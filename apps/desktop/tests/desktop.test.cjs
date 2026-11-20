'use strict';

const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const test = require('node:test');
const vm = require('node:vm');

class Element {
  constructor(tagName) {
    this.tagName = tagName;
    this.children = [];
    this.listeners = new Map();
    this.value = '';
    this.dataset = {};
    this.textContent = '';
    const classes = new Set();
    this.classList = {add: name => classes.add(name), remove: name => classes.delete(name), contains: name => classes.has(name)};
  }
  append(...children) { this.children.push(...children); }
  replaceChildren(...children) { this.children = children; }
  setAttribute(name, value) { this[name] = value; }
  removeAttribute(name) { delete this[name]; }
  closest() { return null; }
  focus() { this.focused = true; }
  addEventListener(name, handler) { this.listeners.set(name, handler); }
  dispatch(name) { return this.listeners.get(name)?.({preventDefault() {}}); }
  get childElementCount() { return this.children.filter(child => child instanceof Element).length; }
}

function descendants(element) {
  return [element, ...element.children.flatMap(child => child instanceof Element ? descendants(child) : [])];
}
const settle = () => new Promise(resolve => setImmediate(resolve));
function deferred() {
  let resolve, reject;
  const promise = new Promise((yes, no) => { resolve = yes; reject = no; });
  return {promise, resolve, reject};
}
function defaultView(view, schedules = []) {
  if (view === 'analytics') return {
    summary: {root_run_states: {}, child_runs: 0, attempts: 0, retries: 0, pending_handoffs: 0, external_attempts: 0},
    cost_page: {currencies: [], unknown_or_partial_calls: 0, not_sent_calls: 0}, calls: [],
  };
  if (view === 'schedules') return {schedules};
  if (view === 'billing') return {calls: 0, groups: []};
  if (view === 'budget') return {policy: null};
  return {};
}

function desktop({scoped, confirm = async () => true, schedules = [], navigation = false, hash = '', mode = 'author'} = {}) {
  const nodes = new Map(), calls = [], confirmations = [];
  const node = id => {
    if (!nodes.has(id)) nodes.set(id, new Element('div'));
    return nodes.get(id);
  };
  const pages = navigation ? ['home','recipes','runs','connections','costs','schedules','activity','recipe','advanced','app-preview'].map(name => {
    const page = node('page-' + name); page.dataset.page = name; return page;
  }) : [];
  const links = pages.map(page => { const link = node('nav-' + page.dataset.page); link.dataset.nav = page.dataset.page; return link; });
  const location = {hash, pathname:'/'};
  const history = {replaceState(_state,_title,url) { location.hash = url.includes('#') ? url.slice(url.indexOf('#')) : ''; }};
  const events = new Map();
  const nativeConfirm = message => { confirmations.push(message); return confirm(message); };
  const context = {
    Error, URLSearchParams,
    location, history,
    crypto: {randomUUID: () => 'synthetic-request-key'},
    document: {
      getElementById: node, createElement: tag => new Element(tag), createTextNode: text => text,
      querySelector: node, querySelectorAll: selector => selector === '[data-page]' ? pages : selector === '[data-nav]' ? links : [], body: {dataset: {mode}},
    },
    window: {
      addEventListener: (name, handler) => events.set(name, handler), scrollTo() {},
      // Tauri's dialog plugin also installs this asynchronous browser global.
      confirm: nativeConfirm,
      __TAURI__: {
        core: {invoke: async (command, args) => {
          if (command === 'desktop_status') return {selected: true, device: 'synthetic'};
          assert.equal(command, 'scoped_call');
          calls.push(JSON.parse(JSON.stringify(args)));
          return scoped ? scoped(args) : defaultView(args.args.view, schedules);
        }},
        dialog: {confirm: nativeConfirm}, event: {listen: async () => {}},
      },
    },
  };
  vm.createContext(context);
  vm.runInContext(fs.readFileSync(path.join(__dirname, '../ui/desktop.js'), 'utf8'), context);
  return {
    node, calls, confirmations, pages, location,
    async visit(hash) { location.hash = hash; await events.get('hashchange')(); },
    call: context.window.aprDesktopCall,
    async inspector() {
      vm.runInContext(fs.readFileSync(path.join(__dirname, '../../../crates/apr-cli/assets/inspector.js'), 'utf8'), context);
      await settle();
      assert.equal(node('notice').textContent, '', 'synthetic startup should complete');
    },
    submitPeriod() { node('billing-period-form').dispatch('submit'); },
  };
}

test('desktop bridge forwards successful calls without changing arguments or result', async () => {
  const result = {calls: 3, groups: []};
  const ui = desktop({scoped: async () => result});
  assert.equal(await ui.call('inspect', {view: 'billing'}, true), result);
  assert.deepEqual(ui.calls, [{action: 'inspect', args: {view: 'billing'}, detail: true}]);
});

for (const message of ['INTERNAL_ERROR', 'Choose a project in Setup first']) {
  test(`desktop bridge exposes native string rejection: ${message}`, async () => {
    const ui = desktop({scoped: async () => { throw message; }});
    await assert.rejects(ui.call('inspect', {view: 'billing'}, true), error => {
      assert.ok(error instanceof Error);
      assert.equal(error.message, message);
      return true;
    });
  });
}

test('desktop bridge preserves an existing Error and its message', async () => {
  const error = new Error('LOCAL_TASK_FAILED');
  const ui = desktop({scoped: async () => { throw error; }});
  await assert.rejects(ui.call('inspect', {view: 'billing'}, true), found => found === error);
});

for (const [label, value] of [['undefined', undefined], ['unknown payload', {private_payload: 'must-not-be-displayed'}]]) {
  test(`desktop bridge gives a useful generic error for ${label}`, async () => {
    const ui = desktop({scoped: async () => { throw value; }});
    await assert.rejects(ui.call('inspect', {view: 'billing'}, true), error => {
      assert.ok(error instanceof Error);
      assert.equal(error.message, 'Local request failed. Open Setup to check the service.');
      assert.ok(!error.message.includes('must-not-be-displayed'));
      return true;
    });
  });
}

for (const view of ['billing', 'budget']) {
  test(`failed ${view} read shows native error and clears obsolete period results`, async () => {
    const ui = desktop({scoped: async ({args}) => {
      if (args.view === view) throw 'INTERNAL_ERROR';
      return defaultView(args.view);
    }});
    await ui.inspector();
    ui.node('period-totals').append(new Element('article'));
    ui.node('period-budget').textContent = 'Obsolete budget';
    ui.submitPeriod();
    await settle();
    assert.equal(ui.node('notice').textContent, 'INTERNAL_ERROR');
    assert.equal(ui.node('connection').textContent, 'Attention needed');
    assert.equal(ui.node('period-status').textContent, 'Could not read this billing period.');
    assert.equal(ui.node('period-totals').childElementCount, 0);
    assert.equal(ui.node('period-budget').textContent, '');
  });
}

test('an older billing failure cannot overwrite a later successful period', async () => {
  const first = deferred();
  let reads = 0;
  const ui = desktop({scoped: async ({args}) => {
    if (args.view === 'billing' && ++reads === 1) return first.promise;
    return defaultView(args.view);
  }});
  await ui.inspector();
  ui.submitPeriod();
  ui.submitPeriod();
  await settle();
  const latest = ui.node('period-status').textContent;
  assert.match(latest, /^0 recorded calls/);
  first.reject('OLDER_REQUEST_FAILED');
  await settle();
  assert.equal(ui.node('period-status').textContent, latest);
  assert.equal(ui.node('notice').textContent, '');
  assert.equal(ui.node('connection').textContent, 'Local service connected');
});

for (const accepted of [false, true]) {
  test(`schedule deletion waits for native consent (${accepted})`, async () => {
    const consent = deferred();
    const ui = desktop({
      confirm: () => consent.promise,
      schedules: [{schedule_id: 'schedule_synthetic', name: 'Synthetic schedule', state: 'paused'}],
    });
    await ui.inspector();
    const remove = descendants(ui.node('schedule-list')).find(element => element.tagName === 'button' && element.textContent === 'Delete');
    assert.ok(remove);
    const deletion = remove.dispatch('click');
    await settle();
    const writes = () => ui.calls.filter(call => call.action === 'manage' && call.args.action === 'schedule.delete');
    assert.equal(ui.confirmations.length, 1);
    assert.equal(writes().length, 0, 'pending confirmation must not delete');
    consent.resolve(accepted);
    await deletion;
    await settle();
    assert.equal(writes().length, accepted ? 1 : 0);
    if (accepted) assert.deepEqual(writes()[0], {action: 'manage', args: {action: 'schedule.delete', schedule_id: 'schedule_synthetic'}, detail: true});
  });
}


test('page navigation keeps one section visible, refreshes it and handles history changes', async () => {
  const ui = desktop({navigation:true});
  await ui.inspector();
  const visible = () => ui.pages.filter(page => !page.hidden).map(page => page.dataset.page);
  assert.deepEqual(visible(), ['home']);
  await ui.visit('#runs');
  assert.deepEqual(visible(), ['runs']);
  assert.equal(ui.node('nav-runs')['aria-current'], 'page');
  assert.equal(ui.node('page-title').focused, true);
  assert.equal(ui.calls.filter(call => call.args.view === 'runs').length, 1);
  await ui.visit('#recipes');
  assert.deepEqual(visible(), ['recipes']);
  assert.equal(ui.node('nav-runs')['aria-current'], undefined);
  // Back/Forward hash changes use the same handler without reloading the session.
  await ui.visit('#runs');
  assert.deepEqual(visible(), ['runs']);
  assert.equal(ui.calls.filter(call => call.args.view === 'runs').length, 2);
  await ui.visit('#unrecognized-page');
  assert.deepEqual(visible(), ['home']);
});

test('routing strips session credentials and does not expose author pages in run mode', async () => {
  const ui = desktop({navigation:true, hash:'#session=synthetic-test-session', mode:'run'});
  await ui.inspector();
  assert.equal(ui.location.hash, '#home');
  await ui.visit('#advanced');
  assert.deepEqual(ui.pages.filter(page => !page.hidden).map(page => page.dataset.page), ['home']);
  await ui.visit('#connections');
  assert.equal(ui.location.hash, '#connections');
  assert.equal(ui.calls.at(-1).args.view, 'bindings');
});

for (const [state, report, outcome, expected] of [
  ['evaluated', {result:'fail', eligible:false}, null, 'Candidate did not pass'],
  ['evaluated', {result:'inconclusive', eligible:false, reason:'additional evidence required'}, null, 'More evidence is needed'],
  ['closed', null, 'no_supported_improvement', 'No supported pure change in inspected cases.'],
]) {
  test(`improvement review shows ${outcome || report.result} without changing a recipe`, async () => {
    let unavailable = false;
    const ui = desktop({scoped: async ({args}) => {
      if (args.view === 'improvements') {
        if (unavailable) throw 'SERVICE_UNAVAILABLE';
        return {cycles:[{cycle_id:'cycle-synthetic', baseline_ref:'sha256:baseline', state, result:report?.result, outcome}]};
      }
      if (args.view === 'improvement') return {cycle_id:'cycle-synthetic', state, report, outcome, reason:'No supported pure change in inspected cases.'};
      return defaultView(args.view);
    }});
    await ui.inspector();
    ui.node('process-ref').value = 'sha256:baseline';
    await ui.node('improvement-refresh').dispatch('click');
    const review = descendants(ui.node('improvement-list')).find(node => node.tagName === 'button');
    assert.ok(review);
    await review.dispatch('click');
    const visible = descendants(ui.node('improvement-report')).map(node => node.textContent).join(' ');
    assert.ok(visible.includes(expected), visible);
    assert.equal(ui.calls.filter(call => call.action === 'manage').length, 0);
    unavailable = true;
    await ui.node('improvement-refresh').dispatch('click');
    assert.equal(ui.node('improvement-report').childElementCount, 0, 'refresh clears the obsolete report');
    assert.match(descendants(ui.node('improvement-list')).map(node => node.textContent).join(' '), /Could not load reviews: SERVICE_UNAVAILABLE/);
  });
}
