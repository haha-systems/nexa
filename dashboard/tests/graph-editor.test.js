import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import test from 'node:test';
import { Window } from 'happy-dom';
import { GraphEditor } from '../graph.js';

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), '../..');
const readJson = (path) => JSON.parse(readFileSync(resolve(repoRoot, path), 'utf8'));
const readText = (path) => readFileSync(resolve(repoRoot, path), 'utf8');
const helloDraft = readJson('examples/hello/hello.compose.json');

function memoryFor(name, contract) {
  const metadata = readJson(`examples/hello/memories/${name}/memory.json`);
  return {
    selector: `${metadata.name}@${metadata.version}`,
    name: metadata.name,
    version: metadata.version,
    kind: 'source',
    exports: [contract],
    source: [{ path: metadata.sources[0], text: readText(`examples/hello/memories/${name}/${metadata.sources[0]}`) }],
  };
}

const memories = [
  memoryFor('trim', {
    export_name: 'trim', kind: 'function',
    parameters: [{ name: 'name', type: 'String' }], result_type: 'String',
    effects: [], capability_requirements: [],
  }),
  memoryFor('greet', {
    export_name: 'greet', kind: 'function',
    parameters: [{ name: 'name', type: 'String' }], result_type: 'String',
    effects: [], capability_requirements: [],
  }),
  memoryFor('write', {
    export_name: 'write', kind: 'action',
    parameters: [{ name: 'message', type: 'String' }], result_type: 'Unit',
    effects: ['io.stdout'], capability_requirements: [{ capability: 'console.write', resource: 'stdout' }],
  }),
];

function createDom() {
  const dom = new Window({ url: 'http://nexa.test/' });
  const saved = {
    window: globalThis.window,
    document: globalThis.document,
    requestAnimationFrame: globalThis.requestAnimationFrame,
    ResizeObserver: globalThis.ResizeObserver,
  };
  globalThis.window = dom;
  globalThis.document = dom.document;
  // Layout is not the subject of these tests; preventing scheduled fit calls
  // keeps the semantic assertions deterministic in the DOM emulator.
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

function makeEditor(draft = helloDraft, options = {}, memoryCatalog = memories) {
  const container = document.createElement('div');
  document.body.append(container);
  const editor = new GraphEditor(container, options);
  editor.setGraph(draft, memoryCatalog);
  return { container, editor };
}

function portButton(container, nodeId, direction, portName) {
  const card = container.querySelector(`.nx-node-card[data-node-id="${nodeId}"]`);
  assert.ok(card, `node card ${nodeId} exists`);
  const button = [...card.querySelectorAll('.nx-port')].find((candidate) =>
    candidate.dataset.portDirection === direction && candidate.dataset.portName === portName);
  assert.ok(button, `${nodeId} ${direction} port ${portName} exists`);
  return button;
}

test('getGraph preserves the example draft and excludes canvas movement', (t) => {
  const env = createDom();
  let changeCount = 0;
  const { container, editor } = makeEditor(helloDraft, { onChange: () => { changeCount += 1; } });
  t.after(() => { editor.destroy(); env.restore(); });

  assert.deepEqual(editor.getGraph(), helloDraft);
  assert.equal(Object.hasOwn(editor.getGraph(), 'positions'), false);
  assert.equal(Object.hasOwn(editor.getGraph().nodes[0], 'position'), false);

  editor.selectNode('clean');
  const card = container.querySelector('.nx-node-card[data-node-id="clean"]');
  const before = card.style.left;
  editor.root.dispatchEvent(new env.dom.KeyboardEvent('keydown', { key: 'ArrowRight', bubbles: true }));

  assert.notEqual(card.style.left, before, 'keyboard arrangement moves the rendered card');
  assert.deepEqual(editor.getGraph(), helloDraft, 'arrangement is not part of the semantic draft');
  assert.equal(changeCount, 0, 'layout-only movement does not emit a draft change');
});

test('adding a real example memory emits a selector/export node', (t) => {
  const env = createDom();
  const changes = [];
  const { editor } = makeEditor({ ...helloDraft, nodes: [], connections: [], order_edges: [] }, {
    onChange: (draft) => changes.push(draft),
  });
  t.after(() => { editor.destroy(); env.restore(); });

  assert.equal(editor.addMemory(memories.find((memory) => memory.selector === 'local.hello.greet@1.0.0')), true);
  assert.deepEqual(editor.getGraph().nodes, [{ id: 'greet', selector: 'local.hello.greet@1.0.0', export: 'greet' }]);
  assert.equal(changes.length, 1);
  assert.deepEqual(changes[0].nodes, editor.getGraph().nodes);
  assert.equal(editor.root.querySelector('.nx-node-card[data-node-id="greet"] .nx-node-export').textContent,
    'local.hello.greet@1.0.0  /  greet');
});

test('deleting a selected node removes its wires and ordering edges and emits the new draft', (t) => {
  const env = createDom();
  const draft = {
    ...helloDraft,
    order_edges: [{ before: 'clean', after: 'greeting' }, { before: 'greeting', after: 'write' }],
  };
  const changes = [];
  const selections = [];
  const { editor } = makeEditor(draft, {
    onChange: (next) => changes.push(next),
    onSelect: (selection) => selections.push(selection),
  });
  t.after(() => { editor.destroy(); env.restore(); });

  editor.selectNode('greeting');
  assert.equal(editor.deleteSelection(), true);

  const current = editor.getGraph();
  assert.deepEqual(current.nodes.map((node) => node.id), ['clean', 'write']);
  assert.equal(current.connections.some((wire) => wire.from.node === 'greeting' || wire.to.node === 'greeting'), false);
  assert.deepEqual(current.order_edges, []);
  assert.equal(changes.length, 1);
  assert.deepEqual(changes[0], current);
  assert.equal(selections.at(-1), null, 'deletion clears the inspector selection');
  assert.equal(editor.root.querySelector('.nx-graph-delete-button').disabled, true);
});

test('port clicks reconnect one producer, retain mismatches visibly, and report selection callbacks', (t) => {
  const env = createDom();
  const draft = {
    ...helloDraft,
    connections: [helloDraft.connections[1]],
    order_edges: [],
  };
  const changes = [];
  const selections = [];
  const { container, editor } = makeEditor(draft, {
    onChange: (next) => changes.push(next),
    onSelect: (selection) => selections.push(selection),
  });
  t.after(() => { editor.destroy(); env.restore(); });

  // `write.result` is Unit while `greeting.name` is String. The editor keeps
  // the attempted wire for inspection and flags the type mismatch.
  portButton(container, 'write', 'source', 'result').click();
  portButton(container, 'greeting', 'target', 'name').click();
  assert.equal(editor.getGraph().connections.length, 1, 'reconnection replaces the prior producer');
  assert.deepEqual(editor.getGraph().connections[0], {
    from: { kind: 'node', node: 'write', port: 'result' },
    to: { kind: 'node', node: 'greeting', port: 'name' },
  });
  assert.match(editor.root.querySelector('.nx-graph-status-text').textContent, /TYPE MISMATCH/);
  assert.ok(editor.root.querySelector('.nx-wire.is-mismatch'));

  portButton(container, 'clean', 'source', 'result').click();
  portButton(container, 'greeting', 'target', 'name').click();
  assert.equal(editor.getGraph().connections.length, 1, 'the target still has only one producer');
  assert.deepEqual(editor.getGraph().connections[0].from, { kind: 'node', node: 'clean', port: 'result' });
  assert.equal(editor.root.querySelector('.nx-wire.is-mismatch'), null);
  assert.equal(changes.length, 2, 'each successful semantic wire change emits one draft');
  assert.equal(selections.at(-1).kind, 'connection');

  container.querySelector('.nx-node-card[data-node-id="clean"]').click();
  assert.equal(selections.at(-1).kind, 'node');
  assert.equal(selections.at(-1).id, 'clean');
  editor.root.querySelector('.nx-wire').dispatchEvent(new env.dom.MouseEvent('click', { bubbles: true }));
  assert.equal(selections.at(-1).kind, 'connection');
  assert.equal(selections.at(-1).id, '0');
});

test('an exact stored memory digest resolves its alias-backed contract and ports', (t) => {
  const env = createDom();
  let editor;
  t.after(() => { editor?.destroy(); env.restore(); });
  const workspace = readJson('docs/validation/dashboard-workspace.json');
  const memory = workspace.memories.find((entry) => entry.selector === 'local.hello.greet@1.0.0');
  assert.ok(memory?.digest, 'the genuine workspace fixture has the stored memory digest');
  const draft = {
    ...helloDraft,
    nodes: [{ id: 'by-digest', selector: memory.digest, export: memory.exports[0].export_name }],
    connections: [],
    order_edges: [],
  };
  let selected;
  const result = makeEditor(draft, { onSelect: (value) => { selected = value; } }, [memory]);
  editor = result.editor;
  const { container } = result;

  const card = container.querySelector('.nx-node-card[data-node-id="by-digest"]');
  assert.ok(card, 'the digest-addressed node is rendered');
  assert.equal(card.querySelector('.nx-node-unresolved'), null, 'the saved contract is resolved');
  assert.ok([...card.querySelectorAll('.nx-port-label')].some((label) => label.textContent === 'name'));
  card.click();
  assert.equal(selected.kind, 'node');
  assert.equal(selected.memory.selector, 'local.hello.greet@1.0.0');
  assert.equal(selected.memory.digest, memory.digest);
});
