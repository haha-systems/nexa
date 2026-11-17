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

let moduleCase = 0;

function jsonResponse(value, status = 200) {
  return new Response(JSON.stringify(value), {
    status,
    headers: { 'Content-Type': 'application/json' },
  });
}

function installPage(fetchHandler) {
  const dom = new Window({ url: 'http://nexa.test/' });
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

async function startApp(t, fetchHandler) {
  const env = installPage(fetchHandler);
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
