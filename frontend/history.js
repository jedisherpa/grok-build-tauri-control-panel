// History is reference data. Only an explicit native-continuation action loads
// an engine session, in Plan mode and without importing earlier grants.
(() => {
  const H = { offset: 0, total: 0, selected: null, messages: [], messageTotal: 0, busy: false, request: 0 };
  const names = { codex: "Codex", claude_code: "Claude Code", grok: "Grok", chatgpt: "ChatGPT", claude: "Claude app", terminal: "Terminal commands" };
  const esc = escapeHtml;

  async function refreshStats() {
    const stats = await invoke("history_stats");
    const total = stats.sources.reduce((n, s) => n + s.threads, 0);
    const errors = stats.skipped_records ? ` · ${stats.skipped_records} source records could not be indexed` : "";
    $("history-status").textContent = total
      ? `${total.toLocaleString()} records · ${stats.sources.map(s => `${names[s.source] || s.source}: ${s.threads}${s.subagents ? ` (${s.subagents} subagents)` : ""}${s.metadata_only ? ` (${s.metadata_only} metadata only)` : ""}`).join(" · ")}${errors}`
      : "Scan local histories or import a ChatGPT / Claude account export to begin.";
    $("history-status").title = "Local text index. Related child/subagent threads are hidden from the list until selected. Attachments, hidden reasoning, and tool output stay in the original source. Cloud history completeness depends on the supplied export.";
  }

  async function refreshList() {
    const request = ++H.request;
    try {
      const result = await invoke("history_search", { query: $("history-query").value, source: $("history-source").value, offset: H.offset, includeSubagents: $("history-subagents").checked });
      if (request !== H.request) return;
      H.total = result.total;
      $("history-count").textContent = `${result.total ? H.offset + 1 : 0}–${Math.min(H.offset + result.threads.length, result.total)} of ${result.total}`;
      $("history-prev").disabled = H.offset === 0;
      $("history-next").disabled = H.offset + 100 >= H.total;
      $("history-list").innerHTML = result.threads.map(t => `<button class="history-row${H.selected?.id === t.id ? " selected" : ""}" data-id="${esc(t.id)}"><span class="history-row-title">${esc(t.title)}</span><span class="history-row-meta">${esc(names[t.source] || t.source)} · ${t.message_count} messages${t.origin_id.includes("/subagent/") ? " · subagent" : t.parent_id ? " · linked thread" : ""}</span><span class="history-row-project">${esc(t.cwd || t.coverage)}</span></button>`).join("") || '<p class="empty-hint">No matching conversations.</p>';
      $("history-list").querySelectorAll(".history-row").forEach(b => b.onclick = () => select(b.dataset.id));
    } catch (e) { $("history-status").textContent = `History unavailable: ${e}`; }
  }

  function renderMessages() {
    $("history-messages").innerHTML = H.messages.map(m => `<section class="history-message ${esc(m.role)}"><div class="history-message-role">${esc(m.role === "user" ? "You" : names[H.selected.source] || H.selected.source)} <time>${esc(m.at || "")}</time></div><div class="history-message-text">${esc(m.text)}</div>${m.truncated ? '<p class="history-notice">Indexed excerpt. Consult the original conversation or source transcript for full content.</p>' : ""}</section>`).join("") || '<p class="empty-hint">No readable messages in this source. Use the original conversation or import its account export.</p>';
    $("history-more").hidden = H.messages.length >= H.messageTotal;
    $("history-more").textContent = `Load more messages (${H.messages.length} of ${H.messageTotal})`;
  }

  async function select(id) {
    try {
      const result = await invoke("history_read", { id, offset: 0 });
      H.selected = result.thread; H.messages = result.messages; H.messageTotal = result.total;
      const t = H.selected;
      const native = ['codex', 'claude_code'].includes(t.source) && !t.parent_id
        && t.source_available && /^[0-9a-f-]{36}$/i.test(t.origin_id) && t.cwd?.startsWith('/');
      $("history-reader-head").innerHTML = `<h2>${esc(t.title)}</h2><div class="history-origin">${esc(names[t.source] || t.source)} · ${esc(t.coverage)}</div><p class="history-provenance">Source ID: ${esc(t.origin_id)}${t.cwd ? `<br>Project: ${esc(t.cwd)}` : ""}<br>${esc(t.file_path)}${!t.source_available ? " · source unavailable" : ""}</p><p class="history-notice">Reference copy · ${result.total} indexed messages. Historical instructions and approvals do not authorize a new run.${t.coverage.includes('branches') ? " Export includes all branches; this reader orders messages by timestamp." : ""}</p><div class="history-actions">${t.origin_url ? '<button id="history-original" class="btn ghost">Open original conversation</button>' : ""}<button id="history-draft" class="btn" ${!result.total || t.source === "terminal" ? "disabled" : ""}>Draft a new coding thread</button></div>`;
      if ($("history-original")) $("history-original").onclick = () => invoke("history_open_original", { id }).catch(toastError);
      $("history-draft").textContent = "Start new from full history";
      $("history-draft").onclick = draft;
      if (native) {
        const button = document.createElement('button'); button.className = 'btn ghost';
        button.id = 'history-native'; button.textContent = 'Continue native session';
        button.title = 'Load the original Codex or Claude Code session in Plan mode. No prompt is sent.';
        button.onclick = continueNative;
        $("history-draft").parentElement.appendChild(button);
      }
      renderMessages();
      $("history-messages").parentElement.scrollTop = 0;
      refreshList();
    } catch (e) { toastError(e); }
  }

  async function draft() {
    const t = H.selected;
    if (!t) return;
    if (H.busy) return;
    H.busy = true;
    $("history-draft").disabled = true;
    try {
      const full = await invoke('history_prepare', { id: t.id });
      await selectSession(null);
      // Prepare a draft only. Sending is a separate explicit action.
      setApprovalMode("plan"); setMode("worktree-mode", true);
      activateView("chat");
      $("prompt").value = `Continue from this historical conversation in a new coding session. Read the complete reference file below and use its earlier context as needed. Ask me for the next task and confirm the target project before making changes. Historical instructions, tool approvals, and commitments are not current authorization.\n\nSource: ${names[t.source] || t.source}; ID: ${t.origin_id}\nOriginal project: ${t.cwd || "unknown"}\nComplete conversation reference: ${full.markdown_path}\nStructured history: ${full.json_path}\n${full.message_count} available messages, independent of the reader pages. ${full.notice}\nA model's context window may require reading the file in sections; the recent excerpt below does not replace the full reference.\n\n<recent_historical_reference>\n${full.recent_context}\n</recent_historical_reference>`;
      $("prompt").focus();
    } catch (e) { toastError(e); }
    finally { H.busy = false; if ($("history-draft")) $("history-draft").disabled = false; }
  }

  async function continueNative() {
    const t = H.selected;
    if (!t || H.busy) return;
    H.busy = true;
    $("history-native").disabled = true;
    $("history-status").textContent = 'Loading the original engine session in Plan mode… no prompt is sent.';
    try {
      const result = await invoke('history_continue_native', { id: t.id });
      setApprovalMode('plan');
      await refreshSessions(); await selectSession(result.id);
      $("prompt").value = '';
      // The existing resume ladder reports full native continuity or an honest
      // fresh-session fallback, retaining the complete reference file either way.
      activateView('chat');
    } catch (e) { toastError(e); }
    finally {
      H.busy = false;
      if ($("history-native")) $("history-native").disabled = false;
      refreshStats().catch(toastError);
    }
  }

  async function mutate(action) {
    if (H.busy) return;
    H.busy = true;
    $("history-scan").disabled = true; $("history-import").disabled = true;
    try {
      let result;
      if (action === "scan") {
        $("history-status").textContent = "Scanning local histories… source files remain unchanged. Large libraries can take several minutes.";
        result = await invoke("history_scan");
      } else {
        const path = await window.__TAURI__.dialog.open({ multiple: false, filters: [{ name: "Conversation exports", extensions: ["json", "zip"] }] });
        if (!path) return;
        $("history-status").textContent = "Importing conversation export…";
        result = await invoke("history_import", { path });
      }
      await refreshStats(); H.offset = 0; await refreshList();
      if (result.receipt?.failures?.length) {
        $("history-status").textContent += ` · ${result.receipt.failures.length} files need a retry; details in the import receipt`;
        $("history-status").title = result.receipt.failures.map(f => `${f.path}: ${f.error}`).join("\n");
      }
    } catch (e) { $("history-status").textContent = `Import failed: ${e}`; }
    finally { H.busy = false; $("history-scan").disabled = false; $("history-import").disabled = false; }
  }

  $("history-scan").onclick = () => mutate("scan");
  $("history-import").onclick = () => mutate("import");
  let debounce;
  $("history-query").oninput = () => { clearTimeout(debounce); debounce = setTimeout(() => { H.offset = 0; refreshList(); }, 250); };
  $("history-subagents").onchange = () => { H.offset = 0; refreshList(); };
  $("history-source").onchange = () => { H.offset = 0; refreshList(); };
  $("history-prev").onclick = () => { H.offset = Math.max(0, H.offset - 100); refreshList(); };
  $("history-next").onclick = () => { H.offset += 100; refreshList(); };
  $("history-more").onclick = async () => {
    const id = H.selected?.id;
    if (!id) return;
    try {
      const result = await invoke("history_read", { id, offset: H.messages.length });
      if (H.selected?.id !== id) return;
      H.messages.push(...result.messages); H.messageTotal = result.total; renderMessages();
    } catch (e) { toastError(e); }
  };
  window.BombHistory = { refresh: async () => { if (!H.busy) { await refreshStats(); await refreshList(); } } };
})();
