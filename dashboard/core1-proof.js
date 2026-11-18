const byId = (id) => document.getElementById(id);
let loading = false;
let loaded = false;

function element(tag, text, className) {
  const item = document.createElement(tag);
  if (text !== undefined) item.textContent = text;
  if (className) item.className = className;
  return item;
}

function memoryName(memory) {
  return memory.module.split('.').at(-1);
}

function details(label, contents) {
  const item = element('details');
  item.append(element('summary', label), element('pre', contents, 'proof-output'));
  return item;
}

function renderMemories(proof) {
  const selected = byId('core1-memory');
  const graph = byId('core1-graph');
  const { memories } = proof;
  const cards = new Map();
  const layers = new Map();

  function place(digest, depth) {
    if ((layers.get(digest) ?? -1) >= depth) return;
    layers.set(digest, depth);
    for (const reference of Object.values(memories[digest].imports)) place(reference, depth + 1);
  }

  function select(digest) {
    const memory = memories[digest];
    for (const [key, button] of cards) button.setAttribute('aria-pressed', String(key === digest));
    const imports = element('ul', undefined, 'proof-imports');
    for (const [alias, target] of Object.entries(memory.imports)) {
      const item = element('li');
      const button = element('button', alias, 'text-link');
      button.type = 'button';
      button.addEventListener('click', () => {
        select(target);
        cards.get(target).focus();
      });
      item.append(button);
      imports.append(item);
    }
    selected.replaceChildren(
      element('h2', memory.module),
      element('code', digest, 'proof-digest'),
      imports.children.length ? imports : element('p', 'No imports'),
      details('Source', proof.sources[digest]),
      details('Checked signature', JSON.stringify(memory.signature, null, 2)),
    );
  }

  place(proof.subject, 0);
  const rows = new Map();
  for (const [digest, depth] of layers) {
    if (!rows.has(depth)) rows.set(depth, element('div', undefined, 'proof-layer'));
    const memory = memories[digest];
    const bindings = Object.values(memory.imports).map((target) => memoryName(memories[target]));
    const card = element('button', undefined, 'proof-node');
    card.type = 'button';
    card.append(
      element('strong', memoryName(memory)),
      element('small', bindings.length ? `Imports ${bindings.join(', ')}` : 'No imports'),
    );
    card.addEventListener('click', () => select(digest));
    cards.set(digest, card);
    rows.get(depth).append(card);
  }
  graph.replaceChildren(...[...rows].sort(([a], [b]) => a - b).map(([, row]) => row));
  select(proof.subject);
}

function wireRequest(hex) {
  const bytes = new Uint8Array(hex.length / 2);
  for (let index = 0; index < bytes.length; index++) {
    bytes[index] = Number.parseInt(hex.slice(index * 2, index * 2 + 2), 16);
  }
  return new TextDecoder().decode(bytes);
}

function bindRun(proof, post) {
  const input = byId('core1-input');
  const run = byId('core1-run');
  const summary = byId('core1-result-summary');
  const output = byId('core1-output');
  const requestPanel = byId('core1-request-panel');
  const reset = () => { input.value = JSON.stringify(proof.input, null, 2); };
  reset();
  byId('core1-reset').addEventListener('click', reset);
  run.addEventListener('click', async () => {
    run.disabled = true;
    summary.textContent = 'Running Core1…';
    output.textContent = '';
    requestPanel.hidden = true;
    try {
      const { value } = await post('/api/core1/run', { input: JSON.parse(input.value) });
      const fields = value.fields;
      if (value.variant === 'complete') {
        const bodyBytes = fields.response.body.length / 2;
        summary.textContent = `HTTP ${fields.response.status} · ${bodyBytes} body bytes`;
      } else {
        summary.textContent = fields.error || 'The response needs more bytes.';
      }
      if (fields.request) {
        byId('core1-request').textContent = wireRequest(fields.request);
        requestPanel.hidden = false;
      }
      output.textContent = JSON.stringify(value, null, 2);
    } catch (error) {
      summary.textContent = error.message;
    } finally {
      run.disabled = false;
    }
  });
}

export async function openCore1Proof(post, readJsonResponse) {
  if (loaded || loading) return;
  loading = true;
  const status = byId('core1-status');
  status.textContent = 'Loading the sealed proof…';
  try {
    const proof = await readJsonResponse(await fetch('/api/core1/proof', { cache: 'no-store' }));
    renderMemories(proof);
    bindRun(proof, post);
    byId('core1-proof').hidden = false;
    status.textContent = '';
    loaded = true;
  } catch (error) {
    status.textContent = error.diagnostic?.message
      || 'Open the local dashboard with --seed-examples to run the Core1 proof.';
  } finally {
    loading = false;
  }
}
