'use strict';

const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const test = require('node:test');
const vm = require('node:vm');

// Exercise the real bundled host bridge. Native WebView rendering/navigation
// and opaque-origin enforcement still require the separate real-app smoke.
function host() {
  const value = {
    document_url: 'cookbook-app://localhost/synthetic-preview/document',
    html: '<h1>Synthetic App</h1>',
    tool_result: {structuredContent: {value: 7}},
  };
  const listeners = new Map(), sent = [];
  const frame = {
    attributes: {}, removed: false,
    setAttribute(name, value) { this.attributes[name] = value; },
    remove() { this.removed = true; },
    contentWindow: {postMessage: (data, origin) => sent.push({data: JSON.parse(JSON.stringify(data)), origin})},
  };
  const status = {textContent: 'Loading…'};
  const context = {
    document: {
      getElementById(id) { return id === 'payload' ? {textContent: JSON.stringify(value)} : status; },
      createElement(tag) { assert.equal(tag, 'iframe'); return frame; },
      querySelector(tag) { assert.equal(tag, 'main'); return {append: child => assert.equal(child, frame)}; },
    },
    window: {
      addEventListener: (name, listener) => listeners.set(name, listener),
      removeEventListener: (name, listener) => {
        assert.equal(listeners.get(name), listener);
        listeners.delete(name);
      },
    },
    Blob, URL: {createObjectURL: () => 'blob:null/synthetic', revokeObjectURL() {}},
    performance: {now: () => 0}, navigator: {language: 'en'},
  };
  vm.createContext(context);
  vm.runInContext(fs.readFileSync(path.join(__dirname, '../ui/preview-host.js'), 'utf8'), context);
  return {
    frame, status, sent, value,
    send(data, options = {}) {
      listeners.get('message')?.({source: frame.contentWindow, origin: 'null', ...options, data});
    },
    close() { listeners.get('pagehide')(); },
  };
}

test('native child document loads in the opaque sandbox without a blob navigation', () => {
  const ui = host();
  assert.equal(ui.frame.src, ui.value.document_url);
  assert.equal(ui.frame.attributes.sandbox, 'allow-scripts');
  assert.equal(ui.frame.referrerPolicy, 'no-referrer');
  assert.equal(ui.frame.attributes.allow, "camera 'none'; microphone 'none'; geolocation 'none'; clipboard-read 'none'; clipboard-write 'none'");
});

test('static document load is distinct from App protocol initialization', () => {
  const ui = host();
  const loaded = {type: 'cookbook:document-loaded'};
  ui.send(loaded, {origin: 'cookbook-app://localhost'});
  ui.send(loaded, {source: {}});
  assert.equal(ui.status.textContent, 'Loading…');
  ui.send(loaded);
  assert.equal(ui.status.textContent, 'Read-only App document loaded · App protocol not initialized');
  assert.deepEqual(ui.sent, []);
  ui.send({jsonrpc: '2.0', method: 'ui/notifications/initialized'});
  assert.deepEqual(ui.sent, [], 'tool result requires a successful initialization first');
});

test('initialized child receives only the consented tool result and no host tools', () => {
  const ui = host();
  ui.send({jsonrpc: '2.0', id: 1, method: 'ui/initialize', params: {protocolVersion: '2026-01-26'}});
  assert.equal(ui.sent.length, 1);
  assert.deepEqual(ui.sent[0].data.result.hostCapabilities, {});
  ui.send({jsonrpc: '2.0', method: 'ui/notifications/initialized'});
  assert.deepEqual(ui.sent[1].data, {jsonrpc: '2.0', method: 'ui/notifications/tool-result', params: ui.value.tool_result});
  assert.equal(ui.status.textContent, 'Isolated read-only preview · no host tool access');
  ui.send({type: 'cookbook:document-loaded'});
  assert.equal(ui.status.textContent, 'Isolated read-only preview · no host tool access');
  for (const method of ['tools/call', 'ui/open-link', 'owner_request']) {
    ui.send({jsonrpc: '2.0', id: method, method});
    assert.equal(ui.sent.at(-1).data.error.code, -32601);
  }
});

test('closing the host removes its child and message listener', () => {
  const ui = host();
  ui.close();
  assert.equal(ui.frame.removed, true);
  ui.send({jsonrpc: '2.0', id: 1, method: 'ui/initialize', params: {protocolVersion: '2026-01-26'}});
  assert.deepEqual(ui.sent, []);
});
