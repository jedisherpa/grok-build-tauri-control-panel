import unittest
from structured_recall_questions import prepare_questions
from full_chain_recall_experiment import selected_bag

class Questions(unittest.TestCase):
    def packet(self):
        def reading(rid, actor):
            return {'readingId':rid,'occurrences':[{'atomId':'v','surface':'approve','selectedSourceBindings':[]}, {'atomId':'a','surface':actor,'selectedSourceBindings':[]}]}
        q,c = reading('q','parent'),reading('c','child')
        ev = {'queryEventId':'qe','candidateEventId':'ce','predicateEqual':True,'rolesEqual':False,'polarityEqual':False,'modalityEqual':False,'query':{'sourceEvent':{'predicate':'v','roles':[{'role':'agent','atom_id':'a'}]}},'candidate':{'sourceEvent':{'predicate':'v','roles':[{'role':'agent','atom_id':'a'}]}}}
        pair={'queryReadingId':'q','candidateReadingId':'c','eventLocal':{'eventPairs':[ev],'queryCanonicalGraph':{'links':[],'references':[]},'candidateCanonicalGraph':{'links':[],'references':[]}},'senseSnap':{'contextPinDistance':None}}
        return {'status':'ready','queryProfile':{'readings':[q]},'candidateProfiles':[{'candidateId':'memory','profile':{'readings':[c]}}],'retrieval':{'candidateComparisons':[{'candidateId':'memory','comparison':{'readingPairs':[pair]}}]}}
    def test_questions_keep_event_and_role_evidence(self):
        result=prepare_questions(self.packet())
        self.assertEqual([x['kind'] for x in result['questions']], ['event-participants','polarity','modality'])
        self.assertIn('agent=parent',result['questions'][0]['text'])
        self.assertIn('agent=child',result['questions'][0]['text'])
        self.assertEqual(result['questions'][0]['evidence']['candidateEventId'],'ce')
        self.assertTrue(all(x['sendStatus']=='unsent' for x in result['questions']))
    def test_ambiguous_same_predicate_events_do_not_invent_correspondence(self):
        packet=self.packet()
        events=packet['retrieval']['candidateComparisons'][0]['comparison']['readingPairs'][0]['eventLocal']['eventPairs']
        events.append(dict(events[0],candidateEventId='another'))
        self.assertEqual(prepare_questions(packet)['questions'],[])
    def test_unavailable_and_question_budget(self):
        self.assertEqual(prepare_questions({'status':'unavailable'})['status'],'unavailable')
        self.assertEqual(len(prepare_questions(self.packet(),1)['questions']),1)
        for invalid in (0, True, 1.5):
            with self.assertRaises(ValueError): prepare_questions(self.packet(),invalid)
    def test_concept_counts_are_per_occurrence(self):
        atom={'assertedConceptIds':['concept'],'selectedSourceBindings':[{'sense':{'id':'s1'},'concept_ids':['concept']},{'sense':{'id':'s2'},'concept_ids':['concept']}]}
        self.assertEqual(selected_bag({'readings':[{'occurrences':[atom]}]},concepts=True),['concept'])
        self.assertEqual(selected_bag({'readings':[{'occurrences':[atom]}]}),['s1','s2'])

if __name__=='__main__': unittest.main()
