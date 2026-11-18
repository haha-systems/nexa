import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import test from 'node:test';
import { Window } from 'happy-dom';

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), '../..');
const readJson = (path) => {
  if (path.startsWith('build/')) throw new Error('dashboard tests must use committed fixtures, not ignored build artifacts');
  return JSON.parse(readFileSync(resolve(repoRoot, path), 'utf8'));
};
const readText = (path) => readFileSync(resolve(repoRoot, path), 'utf8');
const workspaceFixture = readJson('docs/validation/dashboard-workspace.json');
const helloExample = workspaceFixture.programs.find((program) => program.id === 'hello');
const savedBuiltProgram = workspaceFixture.memories.find((memory) => memory.program?.entry === helloExample.draft.entry)?.program;
const savedSubject = readJson('docs/validation/dashboard-hello-report.json').subject;
assert.ok(savedBuiltProgram, 'the committed workspace fixture includes the actual saved Hello program');

function core1WorkspaceFixture() {
  const input = readJson('examples/native-http/input.json');
  const requestDigest = `sha256:${'1'.repeat(64)}`;
  const responseDigest = `sha256:${'2'.repeat(64)}`;
  const streamDigest = `sha256:${'3'.repeat(64)}`;
  const resultDigest = `sha256:${'4'.repeat(64)}`;
  const responseType = `example.native_http.response.Response (${responseDigest})`;
  const stepType = `example.native_http.response.Step (${responseDigest})`;
  const exchangeType = `example.native_http.exchange_result.Exchange (${resultDigest})`;
  const memory = (name, digest, imports, filename, exports) => ({
    selector: digest,
    digest,
    name: `example.native_http.${name}`,
    version: '0.3',
    kind: 'source',
    profile: 'core1',
    imports,
    exports,
    source: [{ path: filename, text: readText(`examples/native-http/${filename}`) }],
    program: null,
    evidence: [],
    program_evidence: [],
  });
  const memories = [
    memory('request', requestDigest, {}, 'request.nexa', [{
      export_name: 'encode', kind: 'function',
      parameters: [{ name: 'authority', type: 'Bytes' }, { name: 'target', type: 'Bytes' }],
      result_type: 'Result[Bytes, String]', effects: [], required_bindings: [], capability_requirements: [],
    }]),
    memory('response', responseDigest, {}, 'response.nexa', []),
    memory('response_stream', streamDigest, { response: responseDigest }, 'response_stream.nexa', [{
      export_name: 'decode', kind: 'function',
      parameters: [{ name: 'chunks', type: 'List[Bytes]' }, { name: 'eof', type: 'Bool' }],
      result_type: stepType, effects: [], required_bindings: [], capability_requirements: [],
    }]),
    memory('exchange_result', resultDigest, { response: responseDigest }, 'exchange_result.nexa', [{
      export_name: 'assemble', kind: 'function',
      parameters: [{ name: 'encoded', type: 'Result[Bytes, String]' }, { name: 'decoded', type: stepType }],
      result_type: exchangeType, effects: [], required_bindings: [], capability_requirements: [],
    }]),
  ];
  const connections = [
    ['input', 'authority', 'request', 'authority'],
    ['input', 'target', 'request', 'target'],
    ['input', 'chunks', 'response_stream', 'chunks'],
    ['input', 'eof', 'response_stream', 'eof'],
  ].map(([fromKind, fromPort, toNode, toPort]) => ({
    from: { kind: fromKind, port: fromPort },
    to: { kind: 'node', node: toNode, port: toPort },
  }));
  connections.push(
    { from: { kind: 'node', node: 'request', port: 'result' }, to: { kind: 'node', node: 'exchange_result', port: 'encoded' } },
    { from: { kind: 'node', node: 'response_stream', port: 'result' }, to: { kind: 'node', node: 'exchange_result', port: 'decoded' } },
    { from: { kind: 'node', node: 'exchange_result', port: 'result' }, to: { kind: 'output', port: 'result' } },
  );
  const draft = {
    schema: 'nexa.core1-blueprint-draft/0.1',
    language: '0.3',
    profile: 'core1',
    entry: 'example.native_http.exchange_result.assemble',
    inputs: [
      { name: 'authority', type: 'Bytes' },
      { name: 'target', type: 'Bytes' },
      { name: 'chunks', type: 'List[Bytes]' },
      { name: 'eof', type: 'Bool' },
    ],
    output: { name: 'result', type: exchangeType },
    nodes: [
      { id: 'request', selector: requestDigest, export: 'encode' },
      { id: 'response_stream', selector: streamDigest, export: 'decode' },
      { id: 'exchange_result', selector: resultDigest, export: 'assemble' },
    ],
    connections,
    order_edges: [],
    settings: [],
    requirements: [],
  };
  const program = {
    id: 'core1-http',
    title: 'Core1 HTTP proof',
    description: 'Compose the pinned Core1 HTTP codec memories over fixture bytes.',
    profile: 'core1',
    draft,
    input_ports: draft.inputs,
    inputs: input,
    manifest: { schema: 'nexa.application/0.3', principal: 'app:core1-http', requests: [], policies: [] },
  };
  const compiledProgram = {
    schema: 'nexa.core1-program/0.1',
    language: '0.3',
    profile: 'core1',
    entry: draft.entry,
    inputs: draft.inputs,
    output: draft.output,
    nodes: draft.nodes.map((entry) => ({ id: entry.id, memory: entry.selector, export: entry.export })),
    connections: draft.connections,
    order_edges: [],
  };
  const workspace = {
    ...workspaceFixture,
    programs: [...workspaceFixture.programs, program],
    memories: [...workspaceFixture.memories, ...memories],
  };
  return { workspace, program, draft, memories, compiledProgram, responseType, exchangeType };
}

let moduleCase = 0;

function jsonResponse(value, status = 200) {
  return new Response(JSON.stringify(value), {
    status,
    headers: { 'Content-Type': 'application/json' },
  });
}

function installPage(fetchHandler, url = 'http://nexa.test/') {
  const dom = new Window({ url });
  dom.document.write(readText('dashboard/index.html'));
  dom.document.close();

  const saved = {
    window: globalThis.window,
    document: globalThis.document,
    location: globalThis.location,
    history: globalThis.history,
    localStorage: globalThis.localStorage,
    fetch: globalThis.fetch,
    requestAnimationFrame: globalThis.requestAnimationFrame,
    ResizeObserver: globalThis.ResizeObserver,
  };
  globalThis.window = dom;
  globalThis.document = dom.document;
  globalThis.location = dom.location;
  globalThis.history = dom.history;
  globalThis.localStorage = dom.localStorage;
  globalThis.fetch = fetchHandler;
  globalThis.requestAnimationFrame = () => 1;
  globalThis.ResizeObserver = undefined;
  return {
    dom,
    restore() {
      dom.happyDOM.abort();
      for (const [key, value] of Object.entries(saved)) {
        if (value === undefined) delete globalThis[key];
        else globalThis[key] = value;
      }
    },
  };
}

async function startApp(t, fetchHandler, url) {
  const env = installPage(fetchHandler, url);
  t.after(env.restore);
  await import(new URL(`../app.js?interaction=${++moduleCase}`, import.meta.url));
  await waitFor(() => document.querySelector('#topbar-status')?.textContent !== 'RUNTIME CONNECTING');
  return env;
}

async function waitFor(predicate, message = 'condition did not become true') {
  for (let attempt = 0; attempt < 20; attempt += 1) {
    if (predicate()) return;
    await new Promise((resolvePromise) => setImmediate(resolvePromise));
  }
  assert.fail(message);
}

test('live bootstrap uses the example inputs, starts with no grants, and a semantic graph edit invalidates a genuine build', async (t) => {
  const requests = [];
  const fetchMock = async (path, init = {}) => {
    const url = String(path);
    requests.push({ url, init });
    if (url === '/api/workspace') return jsonResponse(workspaceFixture);
    if (url === '/api/build') {
      const body = JSON.parse(init.body);
      assert.deepEqual(body.draft, workspaceFixture.programs.find((program) => program.id === 'hello').draft,
        'the tested build begins from the checked-in Hello example draft');
      // These are the actual successful build identity and program persisted
      // by the integration run; the test does not synthesize compiler output.
      return jsonResponse({ program: savedBuiltProgram, subject: savedSubject });
    }
    if (url === './workspace-snapshot.json') return jsonResponse(workspaceFixture);
    throw new Error(`Unexpected dashboard request: ${url}`);
  };
  const env = await startApp(t, fetchMock);
  const hello = workspaceFixture.programs.find((program) => program.id === 'hello');

  assert.equal(document.querySelector('#topbar-status').textContent, 'RUNTIME CONNECTED');
  assert.equal(document.querySelector('#rail-kernel-descriptor').textContent, 'std.slice0@0.3.0 · slice0 · http-client/0.1');
  assert.deepEqual(JSON.parse(document.querySelector('#input-json').value), hello.inputs,
    'the live example loads the input values supplied by the API fixture');
  assert.deepEqual(JSON.parse(document.querySelector('#manifest-json').value), hello.manifest);
  const grant = document.querySelector('#grant-list input[name="resource-grant"]');
  assert.ok(grant, 'the sample manifest request is shown as a launch-time grant choice');
  assert.equal(grant.checked, false, 'requested authority is unchecked by default');
  assert.equal(document.querySelector('#build-program').disabled, false);
  assert.equal(document.querySelector('#seal-program').disabled, true);

  document.querySelector('#build-program').click();
  await waitFor(() => document.querySelector('#build-state').textContent === 'BUILT', 'the genuine saved build result should be accepted');
  assert.equal(document.querySelector('#seal-program').disabled, false, 'a current build enables sealing');
  assert.equal(requests.filter((entry) => entry.url === '/api/build').length, 1);

  // Rewire the String greeting input from a String producer to the Unit output.
  // The connection is retained for inspection and causes a semantic draft edit.
  const source = document.querySelector('#graph-container .nx-node-card[data-node-id="write"] .nx-port[data-port-direction="source"][data-port-name="result"]');
  const target = document.querySelector('#graph-container .nx-node-card[data-node-id="greeting"] .nx-port[data-port-direction="target"][data-port-name="name"]');
  assert.ok(source && target);
  source.click();
  target.click();

  assert.match(document.querySelector('#graph-revision').textContent, /UNBUILT CHANGES/);
  assert.equal(document.querySelector('#build-state').textContent, 'NOT BUILT');
  assert.equal(document.querySelector('#seal-program').disabled, true, 'the previous build cannot seal a changed draft');
  assert.match(document.querySelector('.nx-graph-status-text').textContent, /TYPE MISMATCH/);
  assert.equal(requests.filter((entry) => entry.url === '/api/build').length, 1, 'editing does not silently submit another build');

  // Keep the environment live long enough to ensure the app bootstrap used the
  // intended DOM instance rather than just returning from its module import.
  assert.ok(env.dom.document.querySelector('#graph-container .nx-node-card'));
});

test('snapshot preview keeps browsing controls available but disables runtime actions', async (t) => {
  const fetchMock = async (path) => {
    const url = String(path);
    if (url === '/api/workspace') return jsonResponse({ diagnostic: { code: 'E_OFFLINE', message: 'runtime unavailable' } }, 503);
    if (url === './workspace-snapshot.json') return jsonResponse(workspaceFixture);
    throw new Error(`Unexpected dashboard request: ${url}`);
  };
  await startApp(t, fetchMock);

  assert.equal(document.querySelector('#topbar-status').textContent, 'SNAPSHOT PREVIEW');
  assert.equal(document.querySelector('#build-program').disabled, true);
  assert.equal(document.querySelector('#playground-build').disabled, true);
  assert.equal(document.querySelector('#run-program').disabled, true);
  assert.equal(document.querySelector('#seal-program').disabled, true);
  assert.equal(document.querySelector('#add-memory-button').disabled, false, 'snapshot browsing and graph edits remain available');
  assert.match(document.querySelector('#app-notice').textContent, /Build, run, and seal are disabled/);
});

test('a build diagnostic from the runtime is displayed in the build inspector', async (t) => {
  const fetchMock = async (path) => {
    const url = String(path);
    if (url === '/api/workspace') return jsonResponse(workspaceFixture);
    if (url === '/api/build') {
      return jsonResponse({
        diagnostic: {
          code: 'E_PORT_TYPE',
          message: 'expected String but found Unit',
          path: 'nodes.greeting.name',
        },
      }, 422);
    }
    if (url === './workspace-snapshot.json') return jsonResponse(workspaceFixture);
    throw new Error(`Unexpected dashboard request: ${url}`);
  };
  await startApp(t, fetchMock);

  document.querySelector('#build-program').click();
  await waitFor(() => document.querySelector('#build-state').textContent === 'BUILD FAILED', 'the failed API request should settle');
  assert.equal(document.querySelector('#build-inspection-content .diagnostic-box').textContent,
    'E_PORT_TYPE · expected String but found Unit · nodes.greeting.name');
  assert.match(document.querySelector('#app-notice').textContent, /E_PORT_TYPE/);
  assert.equal(document.querySelector('#seal-program').disabled, true);
});

test('opening the saved Hello memory creates a resolved, example-seeded blueprint draft', async (t) => {
  const fetchMock = async (path) => {
    if (String(path) === '/api/workspace') return jsonResponse(workspaceFixture);
    if (String(path) === './workspace-snapshot.json') return jsonResponse(workspaceFixture);
    throw new Error(`Unexpected dashboard request: ${String(path)}`);
  };
  await startApp(t, fetchMock);

  const savedHello = workspaceFixture.memories.find((memory) => memory.program?.entry === 'example.hello.hello');
  const helloExample = workspaceFixture.programs.find((program) => program.id === 'hello');
  assert.ok(savedHello && helloExample, 'the genuine workspace includes the saved Hello memory and its starting example');

  document.querySelector('.nav-item[data-view="memories"]').click();
  const memoryRow = [...document.querySelectorAll('#memory-list .memory-row')].find((row) =>
    row.querySelector('.memory-row-name')?.textContent === savedHello.name);
  assert.ok(memoryRow, 'the saved Hello memory is listed in Memories');
  memoryRow.click();
  assert.equal(document.querySelector('#memory-detail h2').textContent, savedHello.name);

  const openBlueprint = [...document.querySelectorAll('#memory-detail button')].find((button) => button.textContent === 'Open blueprint');
  assert.ok(openBlueprint, 'the program memory offers an Open blueprint action');
  openBlueprint.click();

  assert.equal(document.querySelector('#view-blueprints').hidden, false);
  assert.equal(document.querySelector('#graph-program-name').textContent, savedHello.name);
  assert.equal(document.querySelector('#program-select').value, `saved:${savedHello.digest}`);
  assert.equal(document.querySelector('#graph-container .nx-node-card').getAttribute('aria-label')?.startsWith('function node'), true);

  const cards = [...document.querySelectorAll('#graph-container .nx-node-card')];
  assert.equal(cards.length, savedHello.program.nodes.length);
  assert.equal(document.querySelectorAll('#graph-container .nx-node-unresolved').length, 0,
    'digest-addressed nodes resolve their saved memory contracts');
  assert.equal(document.querySelectorAll('#graph-container .nx-port.is-unknown').length, 0,
    'all saved program input and output ports have known contracts');
  assert.ok(document.querySelector('#graph-container .nx-node-card[data-node-id="greeting"] .nx-port[data-port-name="name"]'));
  assert.ok(document.querySelector('#graph-container .nx-node-card[data-node-id="write"] .nx-port[data-port-name="message"]'));

  assert.deepEqual(JSON.parse(document.querySelector('#input-json').value), helloExample.inputs,
    'the saved program entry matches and seeds the genuine Hello sample input');
  assert.deepEqual(JSON.parse(document.querySelector('#manifest-json').value), helloExample.manifest,
    'the saved program entry loads the matching example manifest');
  const grant = document.querySelector('#grant-list input[name="resource-grant"][value="stdout"]');
  assert.ok(grant);
  assert.equal(grant.checked, false, 'opening the saved program does not grant requested resources');
});

test('Core1 is an editable profile-filtered blueprint and runs through the shared playground', async (t) => {
  const { workspace, program, draft, memories, compiledProgram, exchangeType } = core1WorkspaceFixture();
  const fixture = readJson('examples/native-http/fixtures.json').cases[0];
  const calls = [];
  const subject = `sha256:${'a'.repeat(64)}`;
  const fetchMock = async (path, init = {}) => {
    const url = String(path);
    if (url === '/api/workspace') return jsonResponse(workspace);
    if (url === '/api/build') {
      const body = JSON.parse(init.body);
      calls.push({ url, body });
      assert.deepEqual(body, { draft }, 'the generic build route receives the editable Core1 blueprint draft');
      return jsonResponse({
        program: compiledProgram,
        subject,
        inspection: {
          profile: 'core1', node_count: draft.nodes.length,
          connection_count: draft.connections.length, inputs: draft.inputs, output: draft.output,
        },
      });
    }
    if (url === '/api/run') {
      const body = JSON.parse(init.body);
      calls.push({ url, body });
      return jsonResponse({
        stdout: '',
        exit_code: 0,
        receipt: {
          schema: 'nexa.core1-receipt/0.1',
          profile: 'core1',
          program: subject,
          invocation_id: 'core1-invocation-1',
          principal: 'app:core1-http',
          decisions: [],
          effects: [],
          outcome: { kind: 'Succeeded', value: fixture.expected },
        },
      });
    }
    if (url === './workspace-snapshot.json') return jsonResponse(workspace);
    throw new Error(`Unexpected dashboard request: ${url}`);
  };
  const env = await startApp(t, fetchMock);

  assert.equal(document.querySelector('[data-view="core1"]'), null, 'Core1 no longer has a separate navigation view');
  assert.equal(document.querySelector('#view-core1'), null, 'the proof page is removed');
  assert.equal(document.querySelector('#program-select').value, 'hello', 'Slice0 remains the default starting selection');
  const selector = document.querySelector('#program-select');
  selector.value = 'core1-http';
  selector.dispatchEvent(new env.dom.Event('change', { bubbles: true }));

  assert.equal(document.querySelector('#graph-program-name').textContent, 'Core1 HTTP proof');
  assert.equal(document.querySelectorAll('#graph-container .nx-node-card').length, 3);
  assert.equal(document.querySelectorAll('#graph-container .nx-node-unresolved').length, 0);
  assert.equal(document.querySelectorAll('#graph-container .nx-wire.is-mismatch').length, 0,
    'nominal port equality uses the full exact type strings');
  const outputType = document.querySelector('#graph-container .nx-node-card[data-node-id="exchange_result"] .nx-port[data-port-direction="source"] .nx-port-type');
  assert.equal(outputType.title, exchangeType, 'the full nominal type remains available to inspect');
  assert.match(outputType.textContent, /…/, 'the graph shortens long nominal digests for display');
  const addMemoryValues = [...document.querySelectorAll('#add-memory-select option')].map((option) => option.value);
  assert.ok(memories.every((memory) => addMemoryValues.includes(memory.digest)), 'Core1 memories are available in its node picker');
  assert.ok(!addMemoryValues.some((value) => workspaceFixture.memories.some((memory) => memory.digest === value)),
    'Slice0 memories are filtered out of the Core1 node picker');

  document.querySelector('#graph-container .nx-node-card[data-node-id="response_stream"]').click();
  assert.match(document.querySelector('#selection-inspector').textContent, /PINNED IMPORTS/);
  assert.ok(document.querySelector('#selection-inspector').textContent.includes(memories[1].digest),
    'the graph inspector shows the exact pinned import digest');

  document.querySelector('[data-view="memories"]').click();
  const streamRow = [...document.querySelectorAll('#memory-list .memory-row')].find((row) =>
    row.querySelector('.memory-row-name')?.textContent === 'example.native_http.response_stream');
  assert.ok(streamRow, 'Core1 source memories appear in the shared memory catalog');
  streamRow.click();
  const importLink = document.querySelector('#memory-detail .memory-import-link');
  assert.ok(importLink, 'the memory detail exposes imported memories as links');
  assert.equal(importLink.title, memories[1].digest);
  importLink.click();
  assert.equal(document.querySelector('#memory-detail h2').textContent, 'example.native_http.response',
    'a pinned import link opens the referenced memory');
  assert.ok([...document.querySelectorAll('#memory-detail .source-block summary')].some((summary) => summary.textContent === 'response.nexa'));

  document.querySelector('[data-view="blueprints"]').click();
  const core1RequestMemory = memories[0];
  document.querySelector('#add-memory-select').value = core1RequestMemory.digest;
  document.querySelector('#add-memory-button').click();
  assert.equal(document.querySelectorAll('#graph-container .nx-node-card').length, 4,
    'the normal graph editor can add a pinned Core1 memory node');
  document.querySelector('.nx-graph-delete-button').click();
  assert.equal(document.querySelectorAll('#graph-container .nx-node-card').length, 3,
    'the normal graph editor can remove that node');

  assert.equal(document.querySelector('#seal-panel').hidden, true, 'Core1 has no unsupported seal action');
  document.querySelector('[data-view="playground"]').click();
  assert.deepEqual(JSON.parse(document.querySelector('#input-json').value), program.inputs,
    'the checked-in Core1 example input loads in the shared playground');
  assert.equal(document.querySelector('#manifest-card').hidden, true);
  assert.equal(document.querySelector('#grant-card').hidden, true);
  assert.equal(document.querySelector('#playground-build').disabled, false, 'Core1 keeps the shared Build control visible');
  assert.equal(document.querySelector('#run-program').disabled, false, 'Run uses the shared build-then-invoke flow');

  document.querySelector('#playground-build').click();
  await waitFor(() => document.querySelector('#build-state').textContent === 'BUILT');
  assert.equal(document.querySelector('#run-program').disabled, false);
  document.querySelector('#run-program').click();
  await waitFor(() => document.querySelector('#receipt-state').textContent === 'SUCCEEDED');
  assert.equal(calls.filter((call) => call.url === '/api/build').length, 1);
  assert.deepEqual(calls.find((call) => call.url === '/api/run').body, {
    program: compiledProgram,
    input: fixture.input,
    manifest: program.manifest,
    allow: [],
  }, 'Core1 uses the shared runner with empty manifest and grant data');
  assert.deepEqual(JSON.parse(document.querySelector('#stdout-output').textContent), fixture.expected,
    'the typed Core1 return value is the primary receipt output');
  assert.match(document.querySelector('#receipt-meta').textContent, /HTTP 404 · 6 body bytes/);
  const requestDetail = [...document.querySelectorAll('#receipt-meta details')].find((details) => details.textContent.includes('ENCODED GET REQUEST'));
  assert.ok(requestDetail);
  assert.match(requestDetail.textContent, /GET \/missing\?x=%2F HTTP\/1\.1/);
});

test('legacy #core1 deep links open the Blueprint view and select the Core1 program', async (t) => {
  const { workspace } = core1WorkspaceFixture();
  await startApp(t, async (path) => {
    if (String(path) === '/api/workspace') return jsonResponse(workspace);
    if (String(path) === './workspace-snapshot.json') return jsonResponse(workspace);
    throw new Error(`Unexpected dashboard request: ${String(path)}`);
  }, 'http://nexa.test/#core1');

  assert.equal(document.querySelector('#view-blueprints').hidden, false);
  assert.equal(document.querySelector('#program-select').value, 'core1-http');
  assert.equal(location.hash, '#blueprints');
});
