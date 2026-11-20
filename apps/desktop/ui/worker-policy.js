'use strict';
(() => {
  const $ = id => document.getElementById(id);
  const call = request => window.__TAURI__.core.invoke('owner_request', {request});
  let current = null;
  const message = error => {
    $('worker-result').textContent = typeof error === 'string' ? error : (error?.message || 'Could not save execution limits.');
  };
  const number = (node, label) => {
    const value = Number(node.value);
    if (!Number.isInteger(value) || value < 1 || value > 64) throw Error(`${label} must be a whole number from 1 to 64.`);
    return value;
  };
  const row = (name = '', limit = 1) => {
    const wrap = document.createElement('div');
    wrap.className = 'actions worker-connection';
    const label = document.createElement('label');
    label.textContent = 'Connection name';
    const input = document.createElement('input');
    input.type = 'text';
    input.maxLength = 128;
    input.placeholder = 'Approved connection name';
    input.value = name;
    label.append(input);
    const capLabel = document.createElement('label');
    capLabel.textContent = 'Jobs';
    const cap = document.createElement('input');
    cap.type = 'number';
    cap.min = '1';
    cap.max = '64';
    cap.step = '1';
    cap.value = String(limit);
    capLabel.append(cap);
    const remove = document.createElement('button');
    remove.type = 'button';
    remove.textContent = 'Remove';
    remove.onclick = () => wrap.remove();
    wrap.append(label, capLabel, remove);
    $('worker-connections').append(wrap);
  };
  const setPreset = value => {
    const preset = ['4', '8', '16', '32'].includes(String(value)) ? String(value) : 'custom';
    $('worker-preset').value = preset;
    $('worker-custom-label').hidden = preset !== 'custom';
    $('worker-custom').value = String(value);
  };
  const render = policy => {
    current = policy;
    setPreset(policy.max_workers);
    $('worker-local').value = String(policy.max_local_workers);
    $('worker-connections').replaceChildren();
    for (const [name, limit] of Object.entries(policy.per_connection)) row(name, limit);
    $('worker-status').textContent = `Limit: ${policy.max_workers} simultaneous jobs · ${policy.max_local_workers} local jobs`;
    $('worker-result').textContent = '';
  };
  const read = async () => {
    const status = await call({action: 'worker_status'});
    render(status.policy);
    $('worker-status').textContent = `${status.running} running · about ${status.queued} queued steps · limit ${status.policy.max_workers} simultaneous jobs (${status.running_local} local, limit ${status.policy.max_local_workers})`;
  };
  const draft = () => {
    if (!current) throw Error('Refresh limits before saving.');
    const max_workers = number($('worker-preset').value === 'custom' ? $('worker-custom') : $('worker-preset'), 'Simultaneous jobs');
    const max_local_workers = number($('worker-local'), 'Local jobs');
    if (max_local_workers > max_workers) throw Error('Local jobs cannot exceed simultaneous jobs.');
    const per_connection = Object.create(null);
    for (const item of $('worker-connections').children) {
      const [nameInput, capInput] = [...item.querySelectorAll('input')];
      const name = nameInput.value;
      if (!name || name.trim() !== name || /[\x00-\x1f\x7f]/.test(name)) throw Error('Enter an approved connection name without surrounding spaces or control characters.');
      if (Object.hasOwn(per_connection, name)) throw Error(`Duplicate connection: ${name}`);
      const cap = number(capInput, `Limit for ${name}`);
      if (cap > max_workers) throw Error(`Limit for ${name} cannot exceed simultaneous jobs.`);
      per_connection[name] = cap;
    }
    return {revision: current.revision, max_workers, max_local_workers, per_connection};
  };
  $('worker-preset').onchange = () => {
    $('worker-custom-label').hidden = $('worker-preset').value !== 'custom';
  };
  $('worker-add-connection').onclick = () => row('', 1);
  $('worker-reload').onclick = () => read().catch(message);
  $('worker-save').onclick = async () => {
    try {
      const policy = draft();
      const saved = await call({action: 'set_worker_policy', policy, expected_revision: current.revision});
      render(saved);
      $('worker-result').textContent = 'Execution limits saved. Running jobs may finish under the previous limit.';
      try {
        const status = await call({action: 'worker_status'});
        $('worker-status').textContent = `${status.running} running · about ${status.queued} queued steps · limit ${status.policy.max_workers} simultaneous jobs (${status.running_local} local, limit ${status.policy.max_local_workers})`;
      } catch (_) {
        $('worker-status').textContent = `Limit: ${saved.max_workers} simultaneous jobs · ${saved.max_local_workers} local jobs. Refresh to update running and queued counts.`;
      }
    } catch (error) {
      message(error);
    }
  };
  read().catch(message);
})();
