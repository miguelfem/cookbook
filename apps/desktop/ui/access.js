'use strict';
(() => {
  const $ = id => document.getElementById(id), invoke = window.__TAURI__.core.invoke;
  const call = request => invoke('owner_request', {request});
  const act = async fn => { try { await fn(); } catch (e) { $('status').textContent = typeof e === 'string' ? e : e.message; } };
  async function storage() {
    const value = await call({action:'credential_store_status'});
    $('credential-status').textContent = `${value.available_accounts} of ${value.saved_accounts} saved accounts available. File: ${value.path}`;
    $('credential-migrate').hidden = !value.legacy_migration_available;
    $('credential-migration-note').hidden = !value.legacy_migration_available;
  }
  $('credential-refresh').onclick = () => act(storage);
  $('credential-migrate').onclick = () => act(async () => {
    if (!await window.__TAURI__.dialog.confirm('Copy existing saved keys to the private, unencrypted local TOML file? Your operating system may request one-time consent to read the old entries. No keys are shown or sent to your agent.')) return;
    $('credential-migrate').disabled = true;
    $('credential-status').textContent = 'Importing saved keys. Complete any operating-system consent prompts.';
    try {
      const value = await call({action:'migrate_platform_credentials'});
      await storage();
      $('status').textContent = `Imported ${value.imported} saved keys. Future recipe calls use the local file.`;
    } finally { $('credential-migrate').disabled = false; }
  });
  let reviewed = null, generation = 0;
  function invalidate() { generation++; reviewed = null; $('python-consent').checked = false; $('python-approve').disabled = true; $('python-review').hidden = true; }
  async function pythonStatus() {
    invalidate();
    const assignment = $('python-workspace').value;
    if (!assignment) { $('python-status').textContent = 'Select a workspace to review its Python access.'; return; }
    const value = await call({action:'python_status', assignment});
    if (assignment !== $('python-workspace').value) return;
    $('python-status').textContent = value.enabled && value.unchanged
      ? `Approved for this workspace (revision ${value.revision}). Recipes reuse this approval.`
      : value.enabled ? 'The approved interpreter has changed. Review the new version before using it.' : 'Python is not approved for this workspace yet.';
    if (value.interpreter) $('python-interpreter').value = value.interpreter.path;
  }
  async function workspaces() {
    const [data, status] = await Promise.all([call({action:'list_assignments'}), invoke('desktop_status')]);
    const selected = $('python-workspace').value || status.assignment_id || '';
    $('python-workspace').replaceChildren(new Option('Select a workspace', ''));
    for (const a of data.assignments) if (a.state === 'active') $('python-workspace').append(new Option(`${a.name} · ${a.root}`, a.assignment_id));
    $('python-workspace').value = selected;
    await pythonStatus();
  }
  $('python-refresh').onclick = () => act(workspaces);
  $('python-workspace').onchange = () => act(pythonStatus);
  $('python-interpreter').oninput = invalidate;
  $('python-browse').onclick = () => act(async () => {
    const path = await window.__TAURI__.dialog.open({multiple:false, directory:false, title:'Choose the Python interpreter prepared by your agent'});
    if (typeof path === 'string') { $('python-interpreter').value = path; invalidate(); }
  });
  $('python-preview').onclick = () => act(async () => {
    invalidate();
    const assignment = $('python-workspace').value, interpreter = $('python-interpreter').value.trim(), current = generation;
    if (!assignment) throw Error('Select a workspace first.');
    const pin = await call({action:'preview_python', interpreter});
    if (generation !== current) return;
    reviewed = {assignment, interpreter, sha256:pin.sha256};
    $('python-review').textContent = `Interpreter: ${pin.path}\nResolved executable: ${pin.resolved_path}\nSHA-256: ${pin.sha256}\n\nNative Python has your operating-system access. Packages are not frozen by this executable check.`;
    $('python-review').hidden = false;
  });
  $('python-consent').onchange = () => { $('python-approve').disabled = !reviewed || !$('python-consent').checked; };
  $('python-approve').onclick = () => act(async () => {
    if (!reviewed || !$('python-consent').checked) throw Error('Review the interpreter and confirm workspace access first.');
    $('python-approve').disabled = true;
    await call({action:'grant_python', ...reviewed});
    await pythonStatus();
    $('status').textContent = 'Python approved for this workspace. New recipe runs reuse it. Older runs with a different frozen grant still need explicit review.';
  });
  window.addEventListener('hashchange', () => {
    if (location.hash === '#library') act(storage);
    if (location.hash === '#python') act(workspaces);
  });
  act(storage); act(workspaces);
})();
