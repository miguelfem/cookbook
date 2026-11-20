'use strict';
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const test = require('node:test');
const vm = require('node:vm');

class Element {
  constructor(tagName = 'div') {
    this.tagName = tagName;
    this.children = [];
    this.value = '';
    this.textContent = '';
    this.hidden = false;
  }
  append(...children) { this.children.push(...children); }
  replaceChildren(...children) { this.children = children; }
  remove() {}
  querySelectorAll(tag) {
    return this.children.flatMap(child => [
      ...(child.tagName === tag ? [child] : []),
      ...child.querySelectorAll(tag),
    ]);
  }
}

async function screen() {
  const nodes = new Map();
  const node = id => {
    if (!nodes.has(id)) nodes.set(id, new Element());
    return nodes.get(id);
  };
  let policy = {revision: 0, max_workers: 4, max_local_workers: 4, per_connection: {}};
  const writes = [];
  const invoke = async (_command, {request}) => {
    if (request.action === 'worker_status') return {policy, running: 2, running_local: 1, queued: 3};
    if (request.action === 'set_worker_policy') {
      writes.push(request);
      if (request.expected_revision !== policy.revision) throw Error('worker policy changed; reload Settings before saving');
      policy = {...request.policy, revision: policy.revision + 1};
      return policy;
    }
    throw Error('Unexpected request');
  };
  const context = {document: {getElementById: node, createElement: tag => new Element(tag)}, window: {__TAURI__: {core: {invoke}}}};
  vm.createContext(context);
  vm.runInContext(fs.readFileSync(path.join(__dirname, '../ui/worker-policy.js'), 'utf8'), context);
  await new Promise(resolve => setImmediate(resolve));
  return {node, writes, overwrite: next => {policy = next;}};
}

test('loads live status and saves reviewed global, local and connection limits', async () => {
  const ui = await screen();
  assert.match(ui.node('worker-status').textContent, /2 running · about 3 queued steps/);
  ui.node('worker-preset').value = '8';
  ui.node('worker-local').value = '3';
  ui.node('worker-add-connection').onclick();
  const [name, cap] = ui.node('worker-connections').children[0].querySelectorAll('input');
  name.value = 'synthetic-service'; cap.value = '2';
  await ui.node('worker-save').onclick();
  assert.equal(ui.writes.length, 1);
  assert.equal(ui.writes[0].expected_revision, 0);
  assert.equal(ui.writes[0].policy.max_workers, 8);
  assert.equal(ui.writes[0].policy.max_local_workers, 3);
  assert.equal(ui.writes[0].policy.per_connection['synthetic-service'], 2);
  assert.match(ui.node('worker-result').textContent, /saved/);
});

test('blocks invalid limits and a stale Settings edit', async () => {
  const ui = await screen();
  ui.node('worker-local').value = '5';
  await ui.node('worker-save').onclick();
  assert.equal(ui.writes.length, 0);
  assert.match(ui.node('worker-result').textContent, /cannot exceed/);
  ui.node('worker-local').value = '4';
  ui.overwrite({revision: 1, max_workers: 8, max_local_workers: 4, per_connection: {}});
  await ui.node('worker-save').onclick();
  assert.equal(ui.writes.length, 1);
  assert.match(ui.node('worker-result').textContent, /reload Settings/);
});
