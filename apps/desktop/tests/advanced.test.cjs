'use strict';

const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const test = require('node:test');
const vm = require('node:vm');

// Exercise the bundled event handlers through their native-command boundary.
// This small DOM fixture covers form values; it does not qualify native WebView
// rendering, permissions, dialogs, or live MCP/provider compatibility.
class Element {
  constructor(tagName) {
    this.tagName = tagName;
    this.children = [];
    this.value = '';
    this.textContent = '';
    if (tagName === 'input') this.checked = false;
  }
  append(...children) { this.children.push(...children); }
  replaceChildren(...children) { this.children = children; }
}

function descendants(element) {
  return [element, ...element.children.flatMap(child => child instanceof Element ? descendants(child) : [])];
}

async function renderRequest(inputRequest, {confirm = async () => true, samplingJob} = {}) {
  const nodes = new Map();
  const node = id => {
    if (!nodes.has(id)) nodes.set(id, new Element('div'));
    return nodes.get(id);
  };
  const sent = [], confirmations = [];
  const item = {
    run_id: 'run_synthetic', step_id: 'request', interaction_id: 'interaction_synthetic',
    input_requests: {answer: inputRequest},
    ...(samplingJob ? {sampling_worker: {connection: 'synthetic-model'}, sampling_jobs: [samplingJob]} : {}),
  };
  const invoke = async (command, args) => {
    if (command === 'desktop_status') return {assignment_id: 'assignment_synthetic'};
    assert.equal(command, 'owner_request');
    const request = JSON.parse(JSON.stringify(args.request));
    if (request.action === 'update_preferences') return {automatic_checks: false};
    if (request.action === 'mcp_interactions') return {items: sent.length ? [] : [item]};
    assert.ok(['respond_mcp', 'reconcile'].includes(request.action));
    sent.push(request);
    return {};
  };
  const context = {
    document: {getElementById: node, createElement: tag => new Element(tag), createTextNode: text => text},
    window: {__TAURI__: {core: {invoke}, dialog: {confirm: async message => {
      confirmations.push(message);
      return confirm(message);
    }}}},
    crypto: {randomUUID: () => 'synthetic-request-key'},
    // A browser-global confirmation must never stand in for the native dialog.
    confirm: () => { throw Error('Unexpected browser confirmation'); },
  };
  vm.createContext(context);
  vm.runInContext(fs.readFileSync(path.join(__dirname, '../ui/advanced.js'), 'utf8'), context);
  await node('mcp-waits-refresh').onclick();
  const elements = () => descendants(node('mcp-waits'));
  return {
    sent, confirmations, node, elements,
    field(name) {
      const label = elements().find(element => element.tagName === 'label' && element.textContent === name);
      assert.ok(label, `Missing field ${name}`);
      return label.children[0];
    },
    async submit() {
      const action = elements().find(element => element.tagName === 'select' && element.children.some(child => child.value === 'accept'));
      action.value = 'accept';
      const consent = elements().find(element => element.tagName === 'label' && element.className === 'consent');
      consent.children[0].checked = true;
      await elements().find(element => element.tagName === 'button' && element.textContent === 'Submit reviewed response').onclick();
    },
  };
}

function form(properties, required = []) {
  return {method: 'elicitation/create', params: {mode: 'form', requestedSchema: {
    type: 'object', properties, required, additionalProperties: false,
  }}};
}

for (const value of [true, false]) {
  test(`required boolean enum sends selected ${value}`, async () => {
    const ui = await renderRequest(form({flag: {type: 'boolean', enum: [true, false]}}, ['flag']));
    ui.field('flag').value = JSON.stringify(value);
    await ui.submit();
    assert.equal(ui.sent.length, 1);
    assert.deepEqual(ui.sent[0].response.responses.answer, {action: 'accept', content: {flag: value}});
  });
}

test('optional blank enum stays omitted, while selected false and zero remain values', async () => {
  const ui = await renderRequest(form({
    omitted: {type: 'boolean', enum: [true, false]},
    flag: {type: 'boolean', enum: [true, false]},
    count: {type: 'integer', enum: [0, 1]},
    enabled: {type: 'boolean'},
  }));
  ui.field('flag').value = 'false';
  ui.field('count').value = '0';
  ui.field('enabled').checked = true;
  await ui.submit();
  assert.deepEqual(ui.sent[0].response.responses.answer.content, {flag: false, count: 0, enabled: true});
});

test('required blank enum blocks submission before the native command', async () => {
  const ui = await renderRequest(form({flag: {type: 'boolean', enum: [true, false]}}, ['flag']));
  await ui.submit();
  assert.equal(ui.sent.length, 0);
  assert.equal(ui.node('status').textContent, 'flag is required');
});

for (const accepted of [false, true]) {
  test(`sampling reconciliation waits for native confirmation (${accepted})`, async () => {
    let resolveConfirmation;
    const pending = new Promise(resolve => { resolveConfirmation = resolve; });
    const ui = await renderRequest({method: 'sampling/createMessage', params: {maxTokens: 10, messages: []}}, {
      confirm: () => pending,
      samplingJob: {input_id: 'answer', state: 'uncertain', intent_id: 'intent_synthetic'},
    });
    const textareas = ui.elements().filter(element => element.tagName === 'textarea');
    textareas[0].value = 'Synthetic provider fixture confirms no effect';
    const outcome = ui.elements().find(element => element.tagName === 'select');
    outcome.value = 'confirmed_no_effect';
    const button = ui.elements().find(element => element.tagName === 'button' && element.textContent === 'Reconcile worker attempt');
    const click = button.onclick();
    assert.equal(ui.confirmations.length, 1);
    assert.equal(ui.sent.length, 0, 'must wait for the native dialog response');
    resolveConfirmation(accepted);
    await click;
    assert.equal(ui.sent.length, accepted ? 1 : 0);
    if (accepted) assert.deepEqual(ui.sent[0], {
      action: 'reconcile', request: {
        run_id: 'run_synthetic', step_instance_id: 'request', intent_id: 'intent_synthetic',
        outcome: 'confirmed_no_effect', evidence: textareas[0].value, output: null,
      },
    });
  });
}
