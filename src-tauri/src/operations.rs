//! Host effect admission. A committed intent is required before evaluating the
//! effect; an interrupted future leaves a recoverable unresolved operation.
use chrono::Utc;
use grok_events::{ControlEvent, EventBus};
use std::future::Future;
use uuid::Uuid;

pub(crate) async fn recorded<T>(
    bus: &EventBus,
    kind: &str,
    target: String,
    effect: impl Future<Output = Result<T, String>>,
) -> Result<T, String> {
    bus.ensure_durable().map_err(|e| e.to_string())?;
    let operation_id = Uuid::new_v4();
    bus.emit_checked(ControlEvent::HostOperationIntent {
        operation_id, session_id: bus.origin().session_id, kind: kind.into(), target: target.clone(), at: Utc::now(),
    }).map_err(|e| format!("operation was not admitted: {e}"))?;
    let result = effect.await;
    // An error can follow a partial external effect. It never means rollback.
    let outcome = match &result {
        Ok(_) => "observed_success".to_string(),
        Err(error) => format!("failed_or_uncertain: {}",grok_events::diagnostics::sanitize_diagnostic(error)),
    };
    bus.emit_checked(ControlEvent::HostOperationOutcome {
        operation_id, session_id: bus.origin().session_id, kind: kind.into(), target, result: outcome, at: Utc::now(),
    }).map_err(|e| format!("operation {operation_id} outcome could not be committed; inspect recovery before retrying: {e}"))?;
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use grok_persistence::Persistence;
    use std::sync::{Arc, atomic::{AtomicUsize, Ordering}};

    #[tokio::test]
    async fn no_durable_sink_means_no_effect() {
        let count=AtomicUsize::new(0);
        assert!(recorded(&EventBus::new(),"test","fixture".into(),async {count.fetch_add(1,Ordering::SeqCst);Ok(())}).await.is_err());
        assert_eq!(count.load(Ordering::SeqCst),0);
    }
    #[tokio::test]
    async fn observed_effect_has_one_correlated_committed_outcome() {
        let dir=tempfile::tempdir().unwrap();
        let db=Arc::new(Persistence::open(dir.path().join("events.sqlite")).unwrap());
        let bus=EventBus::new();bus.install_sink(db.clone()).unwrap();
        let mut receiver=bus.subscribe_committed();
        recorded(&bus,"test","fixture".into(),async {Ok(())}).await.unwrap();
        let intent=receiver.recv().await.unwrap();let outcome=receiver.recv().await.unwrap();
        let ControlEvent::HostOperationIntent{operation_id,..}=intent.event else {panic!("intent missing")};
        let ControlEvent::HostOperationOutcome{operation_id:observed,result,..}=outcome.event else {panic!("outcome missing")};
        assert_eq!(operation_id,observed);assert_eq!(result,"observed_success");
        assert!(outcome.seq>intent.seq);assert!(receiver.try_recv().is_err());
    }
    #[tokio::test]
    async fn abort_after_intent_reopens_as_unresolved_without_repeating_effect() {
        let dir=tempfile::tempdir().unwrap();
        let path=dir.path().join("events.sqlite");
        let db=Arc::new(Persistence::open(&path).unwrap());
        let bus=Arc::new(EventBus::new());bus.install_sink(db.clone()).unwrap();
        let mut receiver=bus.subscribe_committed();let worker_bus=bus.clone();
        let task=tokio::spawn(async move {
            recorded(&worker_bus,"pending_effect","fixture".into(),async {std::future::pending::<Result<(),String>>().await}).await
        });
        let intent=receiver.recv().await.unwrap();
        assert!(matches!(intent.event,ControlEvent::HostOperationIntent{..}));
        task.abort();let _=task.await;
        drop(bus);drop(db);
        let recovered=Persistence::open(path).unwrap().event_snapshot(None,32).unwrap();
        assert_eq!(recovered.operations.len(),1);
        assert!(recovered.operations[0].outcome_seq.is_none());
        assert!(recovered.operations[0].result.is_none());
    }
    #[tokio::test]
    async fn failed_sql_intent_never_evaluates_the_effect() {
        let dir=tempfile::tempdir().unwrap();
        let path=dir.path().join("events.sqlite");
        let db=Arc::new(Persistence::open(&path).unwrap());
        let fault=rusqlite::Connection::open(&path).unwrap();
        fault.execute_batch("CREATE TRIGGER abort_intent BEFORE INSERT ON event_journal BEGIN SELECT RAISE(ABORT,'generated intent fault'); END;").unwrap();
        let bus=EventBus::new();bus.install_sink(db.clone()).unwrap();
        let count=AtomicUsize::new(0);
        assert!(recorded(&bus,"test","fixture".into(),async {count.fetch_add(1,Ordering::SeqCst);Ok(())}).await.is_err());
        assert_eq!(count.load(Ordering::SeqCst),0);assert!(!bus.health().healthy);
        assert!(db.event_snapshot(None,32).unwrap().operations.is_empty());
    }
}
