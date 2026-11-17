import { GraphEditor } from './graph.js';

const $ = (selector, root = document) => root.querySelector(selector);
const $$ = (selector, root = document) => [...root.querySelectorAll(selector)];
const byId = (id) => document.getElementById(id);
const asText = (value, fallback = '—') => value === null || value === undefined || value === '' ? fallback : String(value);
const copyJson = (value) => value == null ? value : JSON.parse(JSON.stringify(value));
const shortDigest = (value) => {
  const text = asText(value, '');
  return text.length > 23 ? `${text.slice(0, 15)}…${text.slice(-7)}` : text;
};

const state = {
  mode: 'loading',
  workspace: null,
  programId: null,
  customProgram: null,
  draft: null,
  draftRevision: 0,
  builtProgram: null,
  builtSubject: null,
  inspection: null,
  builtRevision: null,
  activeManifest: null,
  defaultManifestText: '',
  inputDefaultText: '{}',
  graph: null,
  selectedMemoryDigest: null,
  selectedRunId: null,
  runHistory: [],
  busy: null,
  suppressGraphChange: false,
  notice: null,
};

class ApiFailure extends Error {
  constructor(message, { diagnostic = null, offline = false, status = 0 } = {}) {
    super(message);
    this.name = 'ApiFailure';
    this.diagnostic = diagnostic;
    this.offline = offline;
    this.status = status;
  }
}

function node(tag, className, text) {
  const element = document.createElement(tag);
  if (className) element.className = className;
  if (text !== undefined && text !== null) element.textContent = String(text);
  return element;
}

function putText(target, value, fallback = '—') {
  if (typeof target === 'string') target = byId(target);
  if (target) target.textContent = asText(value, fallback);
}

function diagnosticText(data, fallback = 'The runtime could not complete this request.') {
  const diagnostic = data?.diagnostic ?? data?.error ?? data;
  if (typeof diagnostic === 'string') return diagnostic;
  if (diagnostic && typeof diagnostic === 'object') {
    const message = diagnostic.message ?? diagnostic.detail;
    const code = diagnostic.code ?? diagnostic.kind;
    const path = diagnostic.path ?? diagnostic.location;
    const pieces = [code, message, path].filter((part) => typeof part === 'string' && part.trim());
    if (pieces.length) return pieces.join(' · ');
    try { return JSON.stringify(diagnostic); } catch { /* use fallback */ }
  }
  return fallback;
}

async function readJsonResponse(response) {
  let data;
  try {
    data = await response.json();
  } catch {
    throw new ApiFailure(`The runtime returned an unreadable response (${response.status}).`, { status: response.status });
  }
  if (!response.ok) {
    throw new ApiFailure(diagnosticText(data, `The runtime returned ${response.status}.`), {
      diagnostic: data?.diagnostic ?? null,
      status: response.status,
    });
  }
  return data;
}

async function apiGetWorkspace() {
  let response;
  try {
    response = await fetch('/api/workspace', {
      headers: { 'X-Nexa-Client': 'dashboard', Accept: 'application/json' },
      cache: 'no-store',
    });
  } catch (error) {
    throw new ApiFailure(error?.message || 'The local runtime is unavailable.', { offline: true });
  }
  return readJsonResponse(response);
}

async function apiPost(path, payload) {
  let response;
  try {
    response = await fetch(path, {
      method: 'POST',
      headers: {
        'Content-Type': 'application/json',
        'X-Nexa-Client': 'dashboard',
        Accept: 'application/json',
      },
      body: JSON.stringify(payload),
    });
  } catch (error) {
    throw new ApiFailure(error?.message || 'The local runtime is unavailable.', { offline: true });
  }
  return readJsonResponse(response);
}

async function loadWorkspaceSnapshot() {
  try {
    const response = await fetch('./workspace-snapshot.json', { headers: { Accept: 'application/json' }, cache: 'no-store' });
    if (!response.ok) return null;
    const data = await response.json();
    return validWorkspace(data) ? data : null;
  } catch {
    return null;
  }
}

function validWorkspace(workspace) {
  return Boolean(workspace && typeof workspace === 'object'
    && workspace.kernel && Array.isArray(workspace.memories) && Array.isArray(workspace.programs));
}

function allMemories() { return state.workspace?.memories || []; }
function allPrograms() { return state.workspace?.programs || []; }
function activeProgram() {
  if (state.customProgram && String(state.customProgram.id) === String(state.programId)) return state.customProgram;
  return allPrograms().find((program) => String(program.id) === String(state.programId)) || null;
}
function isPreview() { return state.mode === 'snapshot'; }
function canMutateRuntime() { return state.mode === 'live'; }
function isBuiltForCurrentDraft() { return Boolean(state.builtProgram && state.builtRevision === state.draftRevision); }

function setNotice(message, tone = 'info') {
  state.notice = message ? { message, tone } : null;
  const target = byId('app-notice');
  if (!target) return;
  target.hidden = !message;
  target.dataset.tone = tone;
  target.textContent = message || '';
}

function setMode(mode, workspace = state.workspace) {
  state.mode = mode;
  if (workspace) state.workspace = workspace;
  const copy = {
    live: { label: 'RUNTIME CONNECTED', card: 'CONNECTED', blurb: 'Workspace contracts and stored memories are loaded from your local Nexa runtime.' },
    snapshot: { label: 'SNAPSHOT PREVIEW', card: 'PREVIEW', blurb: 'Browse, edit, and export this workspace snapshot. Runtime actions need a local Nexa server.' },
    offline: { label: 'RUNTIME UNAVAILABLE', card: 'OFFLINE', blurb: 'Start the Nexa dashboard server and reload to connect to your local workspace.' },
    loading: { label: 'RUNTIME CONNECTING', card: 'CONNECTING', blurb: 'Workspace contracts and stored memories load from your Nexa runtime.' },
  }[mode];
  putText('topbar-status', copy.label);
  putText('rail-runtime-status', copy.card);
  putText('rail-kernel-descriptor', workspace?.kernel?.descriptor ? kernelDescriptorText(workspace.kernel.descriptor) : copy.blurb);
  const digest = workspace?.kernel?.digest;
  putText('rail-kernel-digest', digest ? `KERNEL ${shortDigest(digest)}` : mode === 'offline' ? 'No workspace connection' : 'Waiting for workspace');
  const statusClass = mode === 'live' ? 'is-live' : mode === 'snapshot' ? 'is-preview' : mode === 'offline' ? 'is-offline' : '';
  for (const id of ['rail-status-lamp', 'topbar-status-dot']) {
    const statusElement = byId(id);
    if (statusElement) statusElement.className = `${id === 'rail-status-lamp' ? 'status-lamp' : 'topbar-dot'} ${statusClass}`.trim();
  }
  if (mode === 'snapshot') {
    setNotice('Snapshot preview · Editing and JSON export are available. Build, run, and seal are disabled. Run locally to build and execute.', 'info');
  } else if (mode === 'offline') {
    setNotice('Runtime unavailable · Start the Nexa dashboard server, then reload. The dashboard reads workspace state from GET /api/workspace.', 'error');
  } else if (mode === 'live' && state.notice?.message?.startsWith('Runtime unavailable')) {
    setNotice('Connected to the local Nexa runtime.', 'success');
  }
  updateActionAvailability();
}

function currentDraftKey(programId = state.programId) {
  return `nexa.dashboard.draft.v1:${String(programId ?? 'unselected')}`;
}

function storedDraftFor(program) {
  if (!program?.draft || typeof program.draft !== 'object') return null;
  try {
    const stored = localStorage.getItem(currentDraftKey(program.id));
    if (!stored) return copyJson(program.draft);
    const parsed = JSON.parse(stored);
    if (parsed && parsed.schema === program.draft.schema) return parsed;
  } catch { /* Private browsing and malformed local state fall back to API data. */ }
  return copyJson(program.draft);
}

function saveDraftLocally(draft = state.draft) {
  if (!draft) return;
  try { localStorage.setItem(currentDraftKey(), JSON.stringify(draft)); } catch { /* Editing remains in memory when storage is unavailable. */ }
}

function schemaName(memory) {
  return asText(memory?.kind ?? memory?.implementation?.kind ?? memory?.schema?.replace?.('nexa.memory-', '').replace?.('/0.3', ''), 'other');
}

function memoryKind(memory) {
  const value = schemaName(memory).toLowerCase();
  if (value.includes('source')) return 'source';
  if (value.includes('program') || memory?.implementation?.kind === 'program') return 'program';
  return 'other';
}

function getProgramTitle(program) {
  return asText(program?.title ?? program?.name ?? program?.id, 'Untitled program');
}

function programForEntry(entry) {
  return allPrograms().find((program) => program.draft?.entry === entry) || null;
}

function emptyApplicationManifest() {
  return {
    schema: 'nexa.application/0.3',
    principal: 'app:workbench',
    requests: [],
    policies: [],
  };
}

function kernelDescriptorText(descriptor) {
  if (typeof descriptor === 'string') return descriptor;
  if (!descriptor || typeof descriptor !== 'object') return 'Kernel details unavailable';
  const identity = descriptor.name
    ? `${descriptor.name}${descriptor.version ? `@${descriptor.version}` : ''}`
    : 'Kernel';
  const profile = descriptor.profile ?? descriptor.language;
  const extensions = Array.isArray(descriptor.extensions) ? descriptor.extensions : [];
  return [identity, profile, ...extensions].filter(Boolean).join(' · ');
}

function updateProgramSelectors() {
  const programs = allPrograms();
  for (const select of [byId('program-select'), byId('playground-program-select')]) {
    if (!select) continue;
    select.replaceChildren();
    for (const program of programs) {
      const option = node('option', '', getProgramTitle(program));
      option.value = String(program.id);
      select.append(option);
    }
    if (state.customProgram && String(state.customProgram.id) === String(state.programId)) {
      const option = node('option', '', `${getProgramTitle(state.customProgram)} · saved memory`);
      option.value = String(state.customProgram.id);
      select.append(option);
    }
    select.disabled = programs.length === 0 && !(state.customProgram && String(state.customProgram.id) === String(state.programId));
    select.value = state.programId == null ? '' : String(state.programId);
  }
  const memorySelect = byId('add-memory-select');
  if (memorySelect) {
    memorySelect.replaceChildren();
    const placeholder = node('option', '', allMemories().length ? 'Choose a stored memory' : 'No stored memories');
    placeholder.value = '';
    memorySelect.append(placeholder);
    for (const memory of allMemories()) {
      const option = node('option', '', `${asText(memory.name, memory.selector)} · ${asText(memory.version, '')}`.trim());
      option.value = asText(memory.digest ?? memory.selector, '');
      memorySelect.append(option);
    }
    memorySelect.value = '';
    memorySelect.disabled = allMemories().length === 0 || !state.draft;
  }
  putText('overview-program-count', programs.length);
}

function setActiveProgram(programId, { resetRunOutput = true } = {}) {
  const program = allPrograms().find((entry) => String(entry.id) === String(programId));
  if (!program) return;
  state.customProgram = null;
  state.programId = String(program.id);
  state.draftRevision += 1;
  state.builtProgram = null;
  state.builtSubject = null;
  state.inspection = null;
  state.builtRevision = null;
  state.draft = storedDraftFor(program);
  state.activeManifest = program.manifest && typeof program.manifest === 'object' ? copyJson(program.manifest) : null;
  state.defaultManifestText = state.activeManifest ? JSON.stringify(state.activeManifest, null, 2) : '';
  state.inputDefaultText = inputForDraft(state.draft, program.inputs);
  updateProgramSelectors();
  putText('graph-program-name', getProgramTitle(program));
  putText('graph-revision', state.draft ? 'LOCAL DRAFT' : 'NO DRAFT');
  if (state.graph && state.draft) {
    state.suppressGraphChange = true;
    try { state.graph.setGraph(copyJson(state.draft), allMemories()); }
    catch (error) { showGraphDiagnostic(error); }
    finally { state.suppressGraphChange = false; }
    requestAnimationFrame(() => state.graph?.fit());
  } else if (state.graph) {
    state.suppressGraphChange = true;
    try { state.graph.setGraph(null, allMemories()); }
    catch (error) { showGraphDiagnostic(error); }
    finally { state.suppressGraphChange = false; }
  }
  byId('input-json').value = state.inputDefaultText;
  byId('manifest-json').value = state.defaultManifestText;
  renderInputContract(program.input_ports ?? state.draft?.inputs ?? []);
  putText('input-json-status', state.draft?.inputs?.length ? `${state.draft.inputs.length} DECLARED INPUT${state.draft.inputs.length === 1 ? '' : 'S'}` : 'NO INPUTS DECLARED');
  putText('manifest-json-status', state.defaultManifestText ? 'PROGRAM MANIFEST' : 'NO MANIFEST PROVIDED');
  clearVisibleBuildState();
  renderGrantOptions();
  renderSelection(null);
  putText('playground-build-marker', 'Build a draft to continue');
  $('#playground-build-marker').classList.remove('is-built');
  putText('playground-state-label', state.draft ? 'READY WHEN BUILT' : 'NO DRAFT SELECTED');
  if (resetRunOutput) clearReceiptOutput();
  updateActionAvailability();
}

function inputForDraft(draft, sampleInput = null) {
  if (typeof sampleInput === 'string') {
    try { sampleInput = JSON.parse(sampleInput); } catch { sampleInput = null; }
  }
  if (!sampleInput || typeof sampleInput !== 'object' || Array.isArray(sampleInput)) sampleInput = {};
  const input = {};
  for (const port of draft?.inputs || []) {
    if (!port?.name) continue;
    if (Object.hasOwn(sampleInput, port.name)) input[port.name] = sampleInput[port.name];
    else input[port.name] = port.type === 'Unit' ? null : '';
  }
  return JSON.stringify(input, null, 2);
}

function renderInputContract(ports = state.draft?.inputs || []) {
  const values = ports.map((port) => `${asText(port.name)} : ${asText(port.type)}`);
  putText('input-contract-hint', values.length ? `Contract · ${values.join('  /  ')}` : 'This program declares no input ports.');
}

function clearVisibleBuildState() {
  putText('build-state', state.draft ? 'NOT BUILT' : 'NO DRAFT');
  $('#build-state').className = 'build-state';
  byId('build-inspection-content').replaceChildren(
    node('p', 'compile-placeholder', state.draft ? 'Build this draft to resolve selectors and inspect the compiled program.' : 'Choose a program with a draft to inspect its compiled contract.'),
  );
  putText('blueprint-status-label', state.draft ? 'DRAFT WORKSPACE' : 'NO ACTIVE DRAFT');
  putText('playground-build-marker', state.draft ? 'Build a draft to continue' : 'Choose a program with a draft');
  $('#playground-build-marker').classList.remove('is-built');
  updateActionAvailability();
}

function showGraphDiagnostic(error) {
  const text = error instanceof Error ? error.message : String(error);
  setNotice(`The graph editor could not load this draft: ${text}`, 'error');
}

function updateActionAvailability() {
  const currentBuild = isBuiltForCurrentDraft();
  const runtime = canMutateRuntime();
  const hasDraft = Boolean(state.draft);
  byId('build-program').disabled = !runtime || !hasDraft || state.busy !== null;
  byId('playground-build').disabled = !runtime || !hasDraft || state.busy !== null;
  byId('run-program').disabled = !runtime || !activeProgram() || state.busy !== null;
  byId('seal-program').disabled = !runtime || !currentBuild || state.busy !== null;
  byId('add-memory-button').disabled = !state.draft || allMemories().length === 0;
  if (isPreview()) {
    for (const id of ['build-program', 'playground-build', 'run-program', 'seal-program']) {
      const button = byId(id);
      if (button) button.title = 'Run locally to build and execute';
    }
  }
}

function setBusy(action) {
  state.busy = action;
  for (const id of ['build-program', 'playground-build', 'run-program', 'seal-program']) {
    const button = byId(id);
    if (!button) continue;
    if (action && ((action === 'build' && ['build-program', 'playground-build'].includes(id))
      || (action === 'run' && id === 'run-program') || (action === 'seal' && id === 'seal-program'))) {
      button.dataset.idleText ??= button.textContent;
      button.textContent = action === 'build' ? 'Building…' : action === 'run' ? 'Running…' : 'Sealing…';
    } else if (!action && button.dataset.idleText) {
      button.textContent = button.dataset.idleText;
    }
  }
  updateActionAvailability();
}

function loadGraphEditor() {
  const container = byId('graph-container');
  try {
    state.graph = new GraphEditor(container, {
      onChange: (draft) => onGraphChange(draft),
      onSelect: (selection) => renderSelection(selection),
    });
    const program = activeProgram();
    if (state.draft) {
      state.suppressGraphChange = true;
      state.graph.setGraph(copyJson(state.draft), allMemories());
      state.suppressGraphChange = false;
      putText('graph-program-name', getProgramTitle(program));
      requestAnimationFrame(() => state.graph?.fit());
    }
  } catch (error) {
    state.graph = null;
    showGraphDiagnostic(error);
  }
}

function onGraphChange(draftFromEditor) {
  if (state.suppressGraphChange) return;
  const nextDraft = draftFromEditor ?? state.graph?.getGraph?.();
  if (!nextDraft || typeof nextDraft !== 'object') return;
  state.draft = copyJson(nextDraft);
  state.draftRevision += 1;
  state.builtProgram = null;
  state.builtSubject = null;
  state.inspection = null;
  state.builtRevision = null;
  saveDraftLocally();
  putText('graph-revision', 'UNBUILT CHANGES');
  putText('blueprint-status-label', 'DRAFT · CHANGES UNBUILT');
  clearVisibleBuildState();
  renderGrantOptions();
  updateActionAvailability();
}

function switchView(name, { updateHash = true } = {}) {
  if (!['overview', 'memories', 'blueprints', 'playground', 'runs'].includes(name)) return;
  for (const panel of $$('[data-view-panel]')) panel.hidden = panel.dataset.viewPanel !== name;
  for (const button of $$('.nav-item[data-view]')) {
    const active = button.dataset.view === name;
    button.classList.toggle('is-active', active);
    button.setAttribute('aria-current', active ? 'page' : 'false');
  }
  putText('breadcrumb-current', ({ overview: 'Overview', memories: 'Memories', blueprints: 'Blueprints', playground: 'Playground', runs: 'Runs' })[name]);
  if (updateHash && history.replaceState) history.replaceState(null, '', `#${name}`);
  if (name === 'memories') renderMemoryCatalog();
  if (name === 'overview') renderOverview();
  if (name === 'runs') renderRunHistory();
}

function bindNavigation() {
  for (const button of $$('.nav-item[data-view]')) button.addEventListener('click', () => switchView(button.dataset.view));
  for (const button of $$('[data-open-view]')) button.addEventListener('click', () => switchView(button.dataset.openView));
  $('[data-view-link]')?.addEventListener('click', (event) => {
    event.preventDefault();
    switchView('overview');
  });
}

function programRow(program) {
  const row = node('div', 'program-row');
  const button = node('button', 'program-row-button');
  button.type = 'button';
  button.setAttribute('aria-label', `Open blueprint ${getProgramTitle(program)}`);
  const main = node('span', 'program-row-main');
  const symbol = node('span', 'program-symbol', '↗');
  symbol.setAttribute('aria-hidden', 'true');
  const copy = node('span');
  copy.append(node('strong', 'program-name', getProgramTitle(program)));
  copy.append(node('small', 'program-description', asText(program.description, asText(program.id, 'Blueprint draft'))));
  main.append(symbol, copy);
  button.append(main);
  button.addEventListener('click', () => {
    if (program.id != null) setActiveProgram(program.id);
    switchView('blueprints');
  });
  const meta = node('span', 'program-meta', program.draft ? 'DRAFT' : 'NO DRAFT');
  row.append(button, meta);
  return row;
}

function memoryPreviewRow(memory) {
  const row = node('div', 'memory-preview-row');
  const button = node('button', 'memory-preview-button');
  button.type = 'button';
  const main = node('span', 'memory-preview-main');
  const symbol = node('span', 'program-symbol', 'M');
  symbol.setAttribute('aria-hidden', 'true');
  const copy = node('span');
  copy.append(node('strong', 'memory-preview-name', asText(memory.name, memory.selector)));
  copy.append(node('small', 'memory-preview-selector', asText(memory.selector, `${asText(memory.version, '')} · ${asText(memory.digest, '')}`)));
  main.append(symbol, copy);
  button.append(main);
  button.addEventListener('click', () => {
    state.selectedMemoryDigest = memory.digest ?? memory.selector;
    switchView('memories');
    renderMemoryCatalog();
  });
  row.append(button, node('span', 'memory-kind', memoryKind(memory)));
  return row;
}

function renderOverview() {
  const programs = allPrograms();
  const memories = allMemories();
  const statRoot = byId('overview-stats');
  statRoot.replaceChildren();
  const cells = [
    ['MEMORIES', String(memories.length), ''],
    ['BLUEPRINT STARTING POINTS', String(programs.length), ''],
    ['SESSION RUNS', String(state.runHistory.length), ''],
    ['KERNEL', shortDigest(state.workspace?.kernel?.digest ?? ''), 'stat-cell-accent'],
  ];
  for (const [label, value, cls] of cells) {
    const cell = node('div', `stat-cell ${cls}`.trim());
    cell.append(node('span', '', label), node('strong', '', value || '—'));
    statRoot.append(cell);
  }
  const programList = byId('overview-program-list');
  programList.replaceChildren();
  if (programs.length) programs.slice(0, 5).forEach((program) => programList.append(programRow(program)));
  else programList.append(node('p', 'empty-list', 'No blueprint starting points are available in this workspace.'));
  const memoryList = byId('overview-memory-list');
  memoryList.replaceChildren();
  if (memories.length) memories.slice(0, 5).forEach((memory) => memoryList.append(memoryPreviewRow(memory)));
  else memoryList.append(node('p', 'empty-list', 'No stored memories are available in this workspace.'));
  putText('overview-kernel-name', kernelDescriptorText(state.workspace?.kernel?.descriptor));
  putText('overview-kernel-digest', state.workspace?.kernel?.digest);
  putText('overview-network', state.workspace?.network_enabled === true ? 'Enabled in runtime' : state.workspace?.network_enabled === false ? 'Disabled in runtime' : 'Not reported');
  putText('overview-kernel-short', state.workspace?.kernel?.descriptor ? kernelDescriptorText(state.workspace.kernel.descriptor) : 'LOCAL PROGRAMMING ENVIRONMENT');
}

function exportsOf(memory) { return Array.isArray(memory?.exports) ? memory.exports : []; }
function exportName(contract) { return asText(contract?.export_name ?? contract?.name ?? contract?.entry, 'unnamed export'); }
function contractParameters(contract) { return Array.isArray(contract?.parameters) ? contract.parameters : []; }
function contractType(contract) { return asText(contract?.result_type ?? contract?.output?.type ?? contract?.result?.type, 'Unknown'); }

function renderMemoryCatalog() {
  const query = byId('memory-search').value.trim().toLowerCase();
  const kindFilter = byId('memory-kind-filter').value;
  const memories = allMemories();
  const filtered = memories.filter((memory) => {
    const kind = memoryKind(memory);
    if (kindFilter !== 'all' && kind !== kindFilter) return false;
    const haystack = [memory.name, memory.selector, memory.version, memory.digest, memory.description,
      ...exportsOf(memory).map((contract) => exportName(contract))].join(' ').toLowerCase();
    return !query || haystack.includes(query);
  });
  putText('memory-result-count', `${filtered.length} OF ${memories.length} STORED CONTRACT${memories.length === 1 ? '' : 'S'}`);
  const list = byId('memory-list');
  list.replaceChildren();
  if (!filtered.length) {
    list.append(node('p', 'empty-list', memories.length ? 'No memories match this search.' : 'No stored memories are available in this workspace.'));
  } else {
    for (const memory of filtered) {
      const selector = memory.digest ?? memory.selector;
      const selected = selector === state.selectedMemoryDigest;
      const button = node('button', `memory-row${selected ? ' is-selected' : ''}`);
      button.type = 'button';
      button.setAttribute('role', 'listitem');
      button.setAttribute('aria-pressed', selected ? 'true' : 'false');
      button.append(node('strong', 'memory-row-name', asText(memory.name, memory.selector)));
      button.append(node('span', 'memory-row-selector', asText(memory.selector, memory.digest)));
      const meta = node('span', 'memory-row-meta');
      meta.append(node('span', '', memoryKind(memory)), node('span', '', `${exportsOf(memory).length} EXPORT${exportsOf(memory).length === 1 ? '' : 'S'}`));
      button.append(meta);
      button.addEventListener('click', () => {
        state.selectedMemoryDigest = selector;
        renderMemoryCatalog();
      });
      list.append(button);
    }
  }
  const selectedMemory = filtered.find((memory) => (memory.digest ?? memory.selector) === state.selectedMemoryDigest) || filtered[0] || null;
  if (selectedMemory) {
    state.selectedMemoryDigest = selectedMemory.digest ?? selectedMemory.selector;
    renderMemoryDetail(selectedMemory);
  } else {
    byId('memory-detail').replaceChildren(emptyDetail('Select a memory', 'Choose a stored contract to inspect its ports, source and evidence.', 'M'));
  }
}

function emptyDetail(title, body, glyph) {
  const empty = node('div', 'empty-detail');
  empty.append(node('span', 'detail-glyph', glyph), node('h2', '', title), node('p', '', body));
  return empty;
}

function addDetailSection(parent, title, count = null) {
  const section = node('section', 'detail-section');
  const head = node('div', 'detail-section-heading');
  head.append(node('h3', '', title));
  if (count !== null) head.append(node('span', '', String(count)));
  section.append(head);
  parent.append(section);
  return section;
}

function renderMemoryDetail(memory) {
  const root = byId('memory-detail');
  root.replaceChildren();
  const header = node('header', 'detail-top');
  const titleBlock = node('div');
  titleBlock.append(node('span', 'eyebrow', 'STORED MEMORY'));
  titleBlock.append(node('h2', '', asText(memory.name, 'Untitled memory')));
  titleBlock.append(node('div', 'detail-selector', asText(memory.selector, `${asText(memory.name, '')}@${asText(memory.version, '')}`)));
  header.append(titleBlock);
  const badges = node('div');
  badges.append(node('span', 'kind-tag', memoryKind(memory)));
  if (memory.version) badges.append(node('span', 'version-tag', memory.version));
  header.append(badges);
  root.append(header);

  const digestBlock = node('div', 'detail-digest-block');
  digestBlock.append(node('span', '', 'DIGEST'), node('code', '', asText(memory.digest)));
  digestBlock.append(node('span', '', 'IMPLEMENTATION'), node('code', '', asText(memory.implementation?.kind ?? schemaName(memory))));
  root.append(digestBlock);

  const exports = exportsOf(memory);
  const exportSection = addDetailSection(root, 'EXPORT CONTRACTS', exports.length);
  for (const contract of exports) {
    const item = node('div', 'export-contract');
    item.append(node('strong', '', `${exportName(contract)} · ${asText(contract.kind, 'callable')}`));
    item.append(node('span', '', `→ ${contractType(contract)}`));
    const metadata = node('div', 'contract-meta');
    for (const port of contractParameters(contract)) metadata.append(node('span', 'contract-chip', `${asText(port.name)} : ${asText(port.type)}`));
    for (const effect of contract.effects || []) metadata.append(node('span', 'contract-chip', `effect ${effect}`));
    for (const binding of contract.required_bindings || []) metadata.append(node('span', 'contract-chip', `binding ${binding}`));
    item.append(metadata);
    exportSection.append(item);
  }
  if (!exports.length) exportSection.append(node('p', 'empty-message', 'This memory record does not expose export contracts.'));

  if (memory.program && typeof memory.program === 'object') {
    const programSection = addDetailSection(root, 'BLUEPRINT PROGRAM');
    programSection.append(node('p', 'empty-message', 'This memory contains a compiled program. Open it as a draft to inspect and edit its graph.'));
    const openButton = node('button', 'button button-primary', 'Open blueprint');
    openButton.type = 'button';
    openButton.addEventListener('click', () => openStoredProgram(memory));
    programSection.append(openButton);
    const implementationEvidence = Array.isArray(memory.program_evidence) ? memory.program_evidence : [];
    const implementationSection = addDetailSection(root, 'PROGRAM EVIDENCE', implementationEvidence.length);
    if (implementationEvidence.length) {
      implementationSection.append(node('p', 'empty-message', 'Evidence references attached to this program implementation.'));
      const list = node('ul', 'evidence-list');
      for (const reference of implementationEvidence) {
        const value = typeof reference === 'string' ? reference : JSON.stringify(reference);
        list.append(node('li', '', value));
      }
      implementationSection.append(list);
    } else implementationSection.append(node('p', 'empty-message', 'No evidence references are attached to this program implementation.'));
  }

  const sources = Array.isArray(memory.source) ? memory.source : [];
  if (sources.length) {
    const sourceSection = addDetailSection(root, 'SOURCE', sources.length);
    for (const source of sources) {
      const details = node('details', 'source-block');
      const summary = node('summary', '', asText(source.path, 'source file'));
      const pre = node('pre');
      pre.textContent = asText(source.text, '');
      details.append(summary, pre);
      sourceSection.append(details);
    }
  }

  const evidence = Array.isArray(memory.evidence) ? memory.evidence : [];
  const evidenceSection = addDetailSection(root, 'EVIDENCE REFERENCES', evidence.length);
  if (evidence.length) {
    const list = node('ul', 'evidence-list');
    for (const reference of evidence) {
      const value = typeof reference === 'string' ? reference : JSON.stringify(reference);
      list.append(node('li', '', value));
    }
    evidenceSection.append(list);
  } else evidenceSection.append(node('p', 'empty-message', 'No evidence references are attached to this memory.'));
}

function draftFromStoredProgram(program) {
  return {
    schema: 'nexa.blueprint-draft/0.3',
    language: program.language,
    profile: program.profile,
    entry: program.entry,
    inputs: copyJson(program.inputs || []),
    output: copyJson(program.output),
    nodes: (program.nodes || []).map((item) => ({
      id: item.id,
      selector: item.memory,
      export: item.export,
    })),
    connections: copyJson(program.connections || []),
    order_edges: copyJson(program.order_edges || []),
    settings: copyJson(program.settings || []),
    requirements: copyJson(program.requirements || []),
  };
}

function openStoredProgram(memory) {
  const compiledProgram = memory.program;
  if (!compiledProgram || !Array.isArray(compiledProgram.nodes)) return;
  const id = `saved:${memory.digest ?? memory.selector}`;
  const draft = draftFromStoredProgram(compiledProgram);
  const example = programForEntry(compiledProgram.entry);
  state.customProgram = {
    id,
    title: memory.name ?? memory.selector ?? 'Saved program',
    description: 'Draft opened from a stored program memory.',
    draft,
    inputs: example?.inputs ?? null,
    manifest: memory.manifest && typeof memory.manifest === 'object'
      ? copyJson(memory.manifest)
      : copyJson(example?.manifest ?? emptyApplicationManifest()),
    memory,
  };
  state.programId = id;
  state.draftRevision += 1;
  state.draft = draft;
  state.activeManifest = state.customProgram.manifest;
  state.defaultManifestText = JSON.stringify(state.activeManifest, null, 2);
  state.inputDefaultText = inputForDraft(draft, state.customProgram.inputs);
  state.builtProgram = null;
  state.builtSubject = null;
  state.inspection = null;
  state.builtRevision = null;
  try { localStorage.setItem(currentDraftKey(id), JSON.stringify(draft)); } catch { /* Session editing still works without local storage. */ }
  updateProgramSelectors();
  putText('graph-program-name', getProgramTitle(state.customProgram));
  putText('graph-revision', 'OPENED FROM MEMORY');
  if (state.graph) {
    state.suppressGraphChange = true;
    state.graph.setGraph(copyJson(draft), allMemories());
    state.suppressGraphChange = false;
    state.graph.fit();
  }
  byId('input-json').value = state.inputDefaultText;
  byId('manifest-json').value = state.defaultManifestText;
  renderInputContract(draft.inputs);
  putText('input-json-status', `${draft.inputs.length} DECLARED INPUT${draft.inputs.length === 1 ? '' : 'S'}`);
  putText('manifest-json-status', state.defaultManifestText ? 'PROGRAM MANIFEST' : 'NO MANIFEST PROVIDED');
  clearVisibleBuildState();
  renderGrantOptions();
  renderSelection(null);
  putText('playground-state-label', 'READY WHEN BUILT');
  clearReceiptOutput();
  switchView('blueprints');
  setNotice('Saved program opened as a draft. Build it locally before running or sealing.', 'success');
}

function memoryForSelection(selection) {
  if (selection?.memory && typeof selection.memory === 'object') return selection.memory;
  const item = selection?.node ?? selection?.connection?.node ?? null;
  const selector = item?.selector ?? item?.memory ?? selection?.selector;
  const digest = item?.digest ?? selection?.digest ?? selection?.memoryDigest;
  return allMemories().find((memory) => (selector && memory.selector === selector) || (digest && memory.digest === digest)) || null;
}

function addInspectorKv(parent, label, value, asCode = false) {
  const row = node('div', 'inspector-kv');
  row.append(node('span', '', label));
  row.append(node(asCode ? 'code' : 'strong', '', value));
  parent.append(row);
}

function renderPortList(parent, title, ports) {
  const section = node('div', 'compile-section');
  section.append(node('h3', '', title));
  if (!ports?.length) section.append(node('span', 'compile-placeholder', 'No ports'));
  else {
    const list = node('div', 'inspector-port-list');
    for (const port of ports) {
      const item = node('div', 'inspector-port');
      item.append(node('span', '', asText(port.name, 'port')), node('code', '', asText(port.type)));
      list.append(item);
    }
    section.append(list);
  }
  parent.append(section);
}

function renderSelection(selection) {
  const root = byId('selection-inspector');
  root.replaceChildren();
  if (!selection || !selection.kind) {
    const empty = node('div', 'inspector-empty');
    empty.append(node('span', 'inspector-line'), node('h2', '', 'Select a node or port'), node('p', '', 'Contracts, types and authority details appear here.'));
    root.append(empty);
    return;
  }
  const kind = String(selection.kind);
  if (kind === 'node') {
    const draftNode = selection.node ?? selection;
    const memory = memoryForSelection(selection);
    const contract = exportsOf(memory).find((entry) => exportName(entry) === (draftNode.export ?? selection.export)) || null;
    root.append(node('span', 'inspector-type', 'MEMORY NODE'));
    root.append(node('h2', 'inspector-title', asText(draftNode.id ?? selection.id, 'Selected node')));
    root.append(node('p', 'inspector-subtitle', asText(memory?.selector ?? draftNode.selector ?? draftNode.memory)));
    addInspectorKv(root, 'EXPORT', asText(draftNode.export ?? selection.export));
    if (memory?.digest) addInspectorKv(root, 'DIGEST', memory.digest, true);
    if (contract) {
      addInspectorKv(root, 'KIND', asText(contract.kind));
      renderPortList(root, 'INPUT CONTRACT', contractParameters(contract));
      addInspectorKv(root, 'OUTPUT', contractType(contract));
      const effects = contract.effects || [];
      const caps = contract.capability_requirements || [];
      if (effects.length) addInspectorKv(root, 'EFFECTS', effects.join(', '));
      if (caps.length) addInspectorKv(root, 'CAPABILITIES', caps.map((item) => `${item.capability} · ${item.resource}`).join(', '));
    } else addInspectorKv(root, 'CONTRACT', 'Build to resolve stored selector');
    return;
  }
  if (kind === 'connection') {
    const connection = selection.connection ?? selection;
    root.append(node('span', 'inspector-type', 'CONNECTION'));
    root.append(node('h2', 'inspector-title', 'Typed connection'));
    addInspectorKv(root, 'FROM', endpointLabel(connection.from), true);
    addInspectorKv(root, 'TO', endpointLabel(connection.to), true);
    root.append(node('p', 'compile-placeholder', 'Build the draft to check that these ports are compatible.'));
    return;
  }
  if (kind === 'input' || kind === 'output') {
    const selectionNode = selection.node ?? selection;
    const portName = selection.port ?? selectionNode.port ?? selectionNode.name ?? selection.id;
    const port = kind === 'input'
      ? state.draft?.inputs?.find((candidate) => candidate.name === portName)
      : (state.draft?.output?.name === portName ? state.draft.output : null);
    root.append(node('span', 'inspector-type', `${kind.toUpperCase()} PORT`));
    root.append(node('h2', 'inspector-title', asText(portName, `${kind} port`)));
    addInspectorKv(root, 'TYPE', asText(port?.type ?? selection.type));
    addInspectorKv(root, 'DIRECTION', kind === 'input' ? 'Into the program' : 'Program result');
    return;
  }
  root.append(node('span', 'inspector-type', kind.toUpperCase()));
  root.append(node('h2', 'inspector-title', asText(selection.id ?? selection.name, 'Selection')));
  addInspectorKv(root, 'TYPE', asText(selection.type));
}

function endpointLabel(endpoint) {
  if (!endpoint || typeof endpoint !== 'object') return 'Unknown endpoint';
  if (endpoint.kind === 'node') return `${endpoint.node}.${endpoint.port}`;
  return `${endpoint.kind}.${endpoint.port}`;
}

function renderBuildInspection(inspection, subject) {
  const root = byId('build-inspection-content');
  root.replaceChildren();
  if (!inspection || typeof inspection !== 'object') {
    root.append(node('p', 'compile-placeholder', 'The runtime built the program but returned no inspection details.'));
    return;
  }
  const stats = node('div', 'compile-stat-row');
  stats.append(node('span', 'compile-stat', `${asText(inspection.node_count, '—')} NODES`));
  stats.append(node('span', 'compile-stat', `${asText(inspection.connection_count, '—')} CONNECTIONS`));
  stats.append(node('span', 'compile-stat', asText(inspection.profile, 'PROFILE NOT REPORTED')));
  root.append(stats);
  root.append(node('code', 'compile-digest', `SUBJECT ${asText(subject ?? inspection.subject)}`));
  const nodes = Array.isArray(inspection.nodes) ? inspection.nodes : [];
  if (nodes.length) {
    const section = node('section', 'compile-section');
    section.append(node('h3', '', 'RESOLVED NODES'));
    for (const item of nodes) {
      const line = node('div', 'inspector-kv');
      line.append(node('span', '', asText(item.id ?? item.path)), node('strong', '', `${asText(item.export)} · ${shortDigest(item.memory)}`));
      section.append(line);
    }
    root.append(section);
  }
  renderPortList(root, 'PROGRAM INPUTS', inspection.inputs || []);
  const output = inspection.output;
  if (output) {
    const section = node('section', 'compile-section');
    section.append(node('h3', '', 'PROGRAM OUTPUT'));
    section.append(node('span', 'compile-token', `${asText(output.name)} : ${asText(output.type)}`));
    root.append(section);
  }
  renderTokenSection(root, 'EFFECTS', inspection.effects || []);
  renderTokenSection(root, 'CAPABILITY REQUIREMENTS', (inspection.capability_requirements || []).map((item) => `${item.capability} · ${item.resource}`));
  if (Array.isArray(inspection.dependencies) && inspection.dependencies.length) renderTokenSection(root, 'DEPENDENCIES', inspection.dependencies.map(shortDigest));
}

function renderTokenSection(parent, title, values) {
  if (!values?.length) return;
  const section = node('section', 'compile-section');
  section.append(node('h3', '', title));
  for (const value of values) section.append(node('span', 'compile-token', value));
  parent.append(section);
}

async function buildCurrentDraft() {
  if (!canMutateRuntime() || !state.draft || state.busy) return false;
  let draft;
  try { draft = copyJson(state.graph?.getGraph?.() ?? state.draft); }
  catch (error) {
    showBuildError(error.message || 'The graph contains invalid data.');
    return false;
  }
  const revisionAtStart = state.draftRevision;
  const programIdAtStart = state.programId;
  state.draft = draft;
  saveDraftLocally(draft);
  setBusy('build');
  putText('build-state', 'BUILDING');
  $('#build-state').className = 'build-state';
  byId('build-inspection-content').replaceChildren(node('p', 'compile-placeholder', 'Resolving selectors and checking contracts…'));
  setNotice('Building the current blueprint draft with the local runtime…', 'info');
  try {
    const result = await apiPost('/api/build', { draft });
    if (programIdAtStart !== state.programId || revisionAtStart !== state.draftRevision) {
      showBuildError('The draft changed while the runtime was building. Build the current revision again.');
      return false;
    }
    if (!result?.program || !result?.subject) throw new ApiFailure('The runtime response did not include a compiled program and subject digest.');
    state.builtProgram = result.program;
    state.builtSubject = result.subject;
    state.inspection = result.inspection ?? null;
    state.builtRevision = revisionAtStart;
    putText('build-state', 'BUILT');
    $('#build-state').className = 'build-state is-built';
    putText('graph-revision', `BUILT · ${shortDigest(result.subject)}`);
    putText('blueprint-status-label', 'BUILT · CURRENT REVISION');
    putText('playground-build-marker', `Built · ${shortDigest(result.subject)}`);
    $('#playground-build-marker').classList.add('is-built');
    putText('playground-state-label', 'BUILT · READY TO INVOKE');
    renderBuildInspection(state.inspection, state.builtSubject);
    setNotice(`Build complete · ${shortDigest(result.subject)}`, 'success');
    updateActionAvailability();
    return true;
  } catch (error) {
    showBuildError(error instanceof ApiFailure ? error.message : error.message || String(error));
    if (error instanceof ApiFailure && error.offline) setMode('offline');
    return false;
  } finally {
    setBusy(null);
  }
}

function showBuildError(message) {
  state.builtProgram = null;
  state.builtSubject = null;
  state.inspection = null;
  state.builtRevision = null;
  putText('build-state', 'BUILD FAILED');
  $('#build-state').className = 'build-state is-error';
  byId('build-inspection-content').replaceChildren(node('div', 'diagnostic-box', message));
  putText('blueprint-status-label', 'BUILD NEEDS ATTENTION');
  putText('playground-build-marker', 'Current draft not built');
  $('#playground-build-marker').classList.remove('is-built');
  putText('playground-state-label', 'BUILD REQUIRED');
  updateActionAvailability();
  setNotice(message, 'error');
}

function manifestRequests(manifest) {
  return Array.isArray(manifest?.requests) ? manifest.requests.filter((request) => request && typeof request.resource === 'string') : [];
}

function currentManifest() {
  const text = byId('manifest-json').value.trim();
  if (!text) throw new Error('Add an application manifest before running this program.');
  let manifest;
  try { manifest = JSON.parse(text); }
  catch (error) { throw new Error(`Manifest JSON is invalid: ${error.message}`); }
  if (!manifest || typeof manifest !== 'object' || Array.isArray(manifest)) throw new Error('Manifest must be a JSON object.');
  return manifest;
}

function currentInput() {
  const text = byId('input-json').value.trim();
  let input;
  try { input = JSON.parse(text || '{}'); }
  catch (error) { throw new Error(`Input JSON is invalid: ${error.message}`); }
  if (!input || typeof input !== 'object' || Array.isArray(input)) throw new Error('Program input must be a JSON object.');
  const contracts = state.builtProgram?.inputs ?? state.draft?.inputs ?? [];
  if (Object.keys(input).length !== contracts.length) throw new Error('Input fields must match the declared program contract exactly.');
  for (const port of contracts) {
    if (!port?.name) continue;
    if (!Object.hasOwn(input, port.name)) throw new Error(`Input “${port.name}” is required by the program contract.`);
    if (port.type === 'String' && typeof input[port.name] !== 'string') throw new Error(`Input “${port.name}” must be a string.`);
    if (port.type === 'Unit' && input[port.name] !== null) throw new Error(`Input “${port.name}” must be null for Unit.`);
  }
  return input;
}

function renderGrantOptions() {
  const root = byId('grant-list');
  root.replaceChildren();
  let manifest;
  try { manifest = currentManifest(); }
  catch {
    root.append(node('p', 'grant-empty', state.defaultManifestText ? 'Correct the manifest JSON to review its requested resources.' : 'The selected program has no manifest. Add one in the editor to review its requested resources.'));
    return;
  }
  const requests = manifestRequests(manifest);
  const unique = new Map();
  for (const request of requests) {
    const key = request.resource;
    const entry = unique.get(key) || { resource: key, capabilities: [] };
    if (request.capability && !entry.capabilities.includes(request.capability)) entry.capabilities.push(request.capability);
    unique.set(key, entry);
  }
  if (!unique.size) {
    root.append(node('p', 'grant-empty', 'This manifest requests no host resources. The run will use an empty launch-time grant set.'));
    return;
  }
  for (const item of unique.values()) {
    const label = node('label', 'grant-option');
    const checkbox = node('input');
    checkbox.type = 'checkbox';
    checkbox.name = 'resource-grant';
    checkbox.value = item.resource;
    checkbox.addEventListener('change', () => {
      putText('grant-selection-count', `${selectedGrants().length} SELECTED`);
    });
    const copy = node('span');
    copy.append(node('strong', '', item.resource));
    copy.append(node('small', '', item.capabilities.length ? `Requested for ${item.capabilities.join(', ')}` : 'Requested resource'));
    label.append(checkbox, copy);
    root.append(label);
  }
}

function selectedGrants() {
  return $$('input[name="resource-grant"]:checked', byId('grant-list')).map((checkbox) => checkbox.value);
}

function clearReceiptOutput() {
  putText('receipt-state', 'NO INVOCATION');
  $('#receipt-state').className = 'receipt-state';
  putText('stdout-output', 'Run a built program to see its output and receipt.');
  putText('stdout-code', '—');
  byId('receipt-meta').replaceChildren(node('p', 'receipt-empty', 'The runtime returns a structured receipt for every invocation, including denied and failed outcomes.'));
  putText('receipt-json-output', 'No receipt in this session.');
}

function stdoutText(value) {
  if (typeof value === 'string') return value;
  if (Array.isArray(value) && value.every((byte) => Number.isInteger(byte) && byte >= 0 && byte <= 255)) {
    try { return new TextDecoder().decode(new Uint8Array(value)); } catch { return value.join(', '); }
  }
  return value == null ? '' : JSON.stringify(value, null, 2);
}

function outcomeKind(receipt) {
  return receipt?.outcome?.kind ?? receipt?.outcome?.status ?? 'Outcome not reported';
}

function renderReceipt(result, record = null) {
  const receipt = result?.receipt;
  if (!receipt) return;
  const outcome = outcomeKind(receipt);
  const successful = outcome === 'Succeeded' && Number(result.exit_code) === 0;
  putText('receipt-state', outcome.toUpperCase());
  $('#receipt-state').className = `receipt-state ${successful ? 'is-success' : 'is-failure'}`;
  putText('stdout-output', stdoutText(result.stdout));
  putText('stdout-code', `EXIT ${asText(result.exit_code, '—')}`);
  const meta = byId('receipt-meta');
  meta.replaceChildren();
  for (const [label, value, code] of [
    ['INVOCATION', receipt.invocation_id, true],
    ['PROGRAM', receipt.program ?? record?.subject, true],
    ['PRINCIPAL', receipt.principal, false],
    ['OUTCOME', outcome, false],
    ['DECISIONS', Array.isArray(receipt.decisions) ? String(receipt.decisions.length) : null, false],
    ['EFFECTS', Array.isArray(receipt.effects) ? String(receipt.effects.length) : null, false],
  ]) {
    if (value == null) continue;
    const row = node('div', 'receipt-fact');
    row.append(node('span', '', label), node(code ? 'code' : 'strong', '', value));
    meta.append(row);
  }
  const details = receipt.outcome?.message ?? receipt.outcome?.reason;
  if (details) {
    const row = node('div', 'receipt-fact');
    row.append(node('span', '', 'DETAIL'), node('strong', '', details));
    meta.append(row);
  }
  putText('receipt-json-output', JSON.stringify(receipt, null, 2));
}

function updateManifestStatus() {
  const text = byId('manifest-json').value.trim();
  if (!text) {
    putText('manifest-json-status', 'NO MANIFEST PROVIDED');
    renderGrantOptions();
    return;
  }
  try {
    const manifest = JSON.parse(text);
    if (!manifest || typeof manifest !== 'object' || Array.isArray(manifest)) throw new Error('Manifest must be a JSON object.');
    putText('manifest-json-status', `${manifestRequests(manifest).length} RESOURCE REQUEST${manifestRequests(manifest).length === 1 ? '' : 'S'}`);
    renderGrantOptions();
  } catch (error) {
    putText('manifest-json-status', 'INVALID JSON');
    byId('grant-list').replaceChildren(node('p', 'grant-empty', error.message));
  }
}

function showPlayError(message) {
  const target = byId('playground-error');
  target.hidden = false;
  target.textContent = message;
}

function clearPlayError() {
  byId('playground-error').hidden = true;
  byId('playground-error').textContent = '';
}

async function runProgram() {
  if (!canMutateRuntime() || state.busy || !activeProgram()) return;
  clearPlayError();
  if (!isBuiltForCurrentDraft()) {
    const built = await buildCurrentDraft();
    if (!built) {
      showPlayError('Build the current draft successfully before running it.');
      return;
    }
  }
  let input;
  let manifest;
  try {
    input = currentInput();
    manifest = currentManifest();
  } catch (error) {
    showPlayError(error.message);
    return;
  }
  const allow = selectedGrants();
  const programId = state.programId;
  const programTitle = getProgramTitle(activeProgram());
  const subject = state.builtSubject;
  const compiledProgram = state.builtProgram;
  setBusy('run');
  setNotice('Invoking the built program with the selected launch-time grants…', 'info');
  try {
    const result = await apiPost('/api/run', {
      program: compiledProgram,
      input,
      manifest,
      allow,
    });
    if (result?.receipt) {
      const record = {
        id: `${result.receipt.invocation_id ?? 'invocation'}:${state.runHistory.length + 1}`,
        programId,
        programTitle,
        subject,
        input,
        allow: [...allow],
        stdout: result.stdout,
        exit_code: result.exit_code,
        receipt: result.receipt,
      };
      state.runHistory.unshift(record);
      state.selectedRunId = record.id;
      renderReceipt(result, record);
      renderRunHistory();
      const outcome = outcomeKind(result.receipt);
      setNotice(`Invocation recorded · ${outcome}`, outcome === 'Succeeded' && Number(result.exit_code) === 0 ? 'success' : 'error');
    } else {
      throw new ApiFailure('The runtime response did not include an execution receipt.');
    }
  } catch (error) {
    const message = error instanceof ApiFailure ? error.message : error.message || String(error);
    showPlayError(message);
    setNotice(message, 'error');
    if (error instanceof ApiFailure && error.offline) setMode('offline');
  } finally {
    setBusy(null);
  }
}

function receiptIsSuccess(receipt) { return outcomeKind(receipt) === 'Succeeded'; }

function renderRunHistory() {
  putText('run-history-count', `${state.runHistory.length} RECEIPT${state.runHistory.length === 1 ? '' : 'S'} · THIS SESSION`);
  const root = byId('run-history-list');
  root.replaceChildren();
  if (!state.runHistory.length) {
    root.append(node('p', 'empty-list', 'No invocations have returned a receipt in this browser session.'));
    byId('run-detail-panel').replaceChildren(emptyDetail('No receipt selected', 'Select a run to inspect its outcome, decisions and effects.', 'R'));
    return;
  }
  for (const record of state.runHistory) {
    const button = node('button', `run-history-row${record.id === state.selectedRunId ? ' is-selected' : ''}`);
    button.type = 'button';
    const head = node('span', 'run-history-row-head');
    head.append(node('strong', '', record.programTitle));
    head.append(node('span', `run-result-tag ${receiptIsSuccess(record.receipt) ? 'success' : 'failure'}`, outcomeKind(record.receipt)));
    button.append(head);
    button.append(node('small', '', asText(record.receipt.observed_time ?? record.receipt.invocation_id)));
    button.addEventListener('click', () => {
      state.selectedRunId = record.id;
      renderRunHistory();
    });
    root.append(button);
  }
  const selected = state.runHistory.find((record) => record.id === state.selectedRunId) || state.runHistory[0];
  state.selectedRunId = selected.id;
  renderRunDetail(selected);
}

function renderRunDetail(record) {
  const root = byId('run-detail-panel');
  root.replaceChildren();
  root.append(node('span', 'eyebrow', 'EXECUTION RECEIPT'));
  root.append(node('h2', '', record.programTitle));
  root.append(node('p', 'run-summary-meta', `${asText(record.receipt.invocation_id)} · ${asText(record.receipt.observed_time, 'time not reported')}`));
  const facts = [
    ['OUTCOME', outcomeKind(record.receipt)],
    ['EXIT CODE', asText(record.exit_code)],
    ['SUBJECT', asText(record.receipt.program ?? record.subject)],
    ['PRINCIPAL', asText(record.receipt.principal)],
    ['GRANTS', record.allow.length ? record.allow.join(', ') : 'No resources granted'],
  ];
  for (const [label, value] of facts) {
    const row = node('div', 'run-record-row');
    row.append(node('span', '', label), node('code', '', value));
    root.append(row);
  }
  const outcome = record.receipt.outcome;
  if (outcome?.message || outcome?.reason) {
    root.append(node('h3', '', 'OUTCOME DETAIL'));
    root.append(node('p', 'empty-message', outcome.message ?? outcome.reason));
  }
  const decisions = Array.isArray(record.receipt.decisions) ? record.receipt.decisions : [];
  root.append(node('h3', '', `POLICY DECISIONS · ${decisions.length}`));
  for (const decision of decisions) {
    const row = node('div', 'run-record-row');
    row.append(node('span', '', asText(decision.operation)), node('code', '', `${asText(decision.decision)} · ${asText(decision.resource)}`));
    root.append(row);
  }
  const effects = Array.isArray(record.receipt.effects) ? record.receipt.effects : [];
  root.append(node('h3', '', `EFFECTS · ${effects.length}`));
  for (const effect of effects) {
    const row = node('div', 'run-record-row');
    row.append(node('span', '', asText(effect.node_path)), node('code', '', `${asText(effect.operation)} · ${asText(effect.resource)} · ${effect.dispatched ? 'dispatched' : 'not dispatched'}`));
    root.append(row);
  }
  const stdout = node('details', 'source-block');
  stdout.append(node('summary', '', 'STDOUT'));
  stdout.append(node('pre', '', stdoutText(record.stdout)));
  root.append(stdout);
  const full = node('details', 'source-block');
  full.append(node('summary', '', 'FULL RECEIPT JSON'));
  full.append(node('pre', '', JSON.stringify(record.receipt, null, 2)));
  root.append(full);
}

async function sealBuiltProgram() {
  if (!canMutateRuntime() || !isBuiltForCurrentDraft() || state.busy) return;
  const name = byId('seal-name').value.trim();
  const version = byId('seal-version').value.trim();
  const result = byId('seal-result');
  result.textContent = '';
  if (!name || !version) {
    result.style.color = 'var(--red)';
    result.textContent = 'A memory name and version are required.';
    return;
  }
  setBusy('seal');
  setNotice('Saving the built program as an immutable memory…', 'info');
  try {
    const response = await apiPost('/api/seal', { program: state.builtProgram, name, version });
    const digest = response.digest ?? response.memory?.digest;
    if (digest) state.selectedMemoryDigest = digest;
    result.style.color = 'var(--blue-deep)';
    result.textContent = digest ? `Memory saved · ${digest}` : 'Memory saved.';
    setNotice(digest ? `Memory saved · ${shortDigest(digest)}` : 'Memory saved.', 'success');
    await refreshWorkspace({ keepSelection: true, preserveNotice: true });
  } catch (error) {
    const message = error instanceof ApiFailure ? error.message : error.message || String(error);
    result.style.color = 'var(--red)';
    result.textContent = message;
    setNotice(message, 'error');
    if (error instanceof ApiFailure && error.offline) setMode('offline');
  } finally {
    setBusy(null);
  }
}

async function refreshWorkspace({ keepSelection = false, preserveNotice = false } = {}) {
  const priorProgramId = keepSelection ? state.programId : null;
  const priorNotice = preserveNotice ? state.notice : null;
  setMode('loading');
  try {
    const workspace = await apiGetWorkspace();
    if (!validWorkspace(workspace)) throw new ApiFailure('The runtime returned an incomplete workspace document.');
    state.workspace = workspace;
    setMode('live', workspace);
    updateProgramSelectors();
    const selected = priorProgramId && workspace.programs.some((program) => String(program.id) === String(priorProgramId))
      ? priorProgramId : workspace.programs[0]?.id;
    if (selected != null) setActiveProgram(selected, { resetRunOutput: false });
    else {
      state.programId = null;
      state.draft = null;
      state.graph?.setGraph?.(null, allMemories());
      clearVisibleBuildState();
    }
    updateProgramSelectors();
    state.selectedMemoryDigest = state.selectedMemoryDigest ?? workspace.memories[0]?.digest ?? workspace.memories[0]?.selector ?? null;
    renderOverview();
    renderMemoryCatalog();
    renderRunHistory();
    if (priorNotice) setNotice(priorNotice.message, priorNotice.tone);
    else if (!preserveNotice) setNotice(null);
  } catch (error) {
    if (error instanceof ApiFailure && !error.offline) {
      const snapshot = await loadWorkspaceSnapshot();
      if (snapshot) {
        state.workspace = snapshot;
        setMode('snapshot', snapshot);
        updateProgramSelectors();
        if (snapshot.programs[0]) setActiveProgram(snapshot.programs[0].id, { resetRunOutput: false });
        renderOverview();
        renderMemoryCatalog();
        renderRunHistory();
        return;
      }
    }
    const snapshot = await loadWorkspaceSnapshot();
    if (snapshot) {
      state.workspace = snapshot;
      setMode('snapshot', snapshot);
      updateProgramSelectors();
      const selected = priorProgramId && snapshot.programs.some((program) => String(program.id) === String(priorProgramId))
        ? priorProgramId : snapshot.programs[0]?.id;
      if (selected != null) setActiveProgram(selected, { resetRunOutput: false });
      renderOverview();
      renderMemoryCatalog();
      renderRunHistory();
      return;
    }
    state.workspace = null;
    setMode('offline', null);
    updateProgramSelectors();
    renderOverview();
    renderMemoryCatalog();
    renderRunHistory();
    clearVisibleBuildState();
  }
  if (state.graph && state.draft) {
    state.suppressGraphChange = true;
    state.graph.setGraph(copyJson(state.draft), allMemories());
    state.suppressGraphChange = false;
    state.graph.fit();
  }
}

function bindEvents() {
  bindNavigation();
  byId('refresh-workspace').addEventListener('click', () => refreshWorkspace({ keepSelection: true }));
  byId('program-select').addEventListener('change', (event) => setActiveProgram(event.target.value));
  byId('playground-program-select').addEventListener('change', (event) => setActiveProgram(event.target.value));
  byId('memory-search').addEventListener('input', renderMemoryCatalog);
  byId('memory-kind-filter').addEventListener('change', renderMemoryCatalog);
  byId('build-program').addEventListener('click', buildCurrentDraft);
  byId('playground-build').addEventListener('click', async () => {
    const built = await buildCurrentDraft();
    if (!built) showPlayError('Build the current draft successfully before running it.');
  });
  byId('run-program').addEventListener('click', runProgram);
  byId('seal-program').addEventListener('click', sealBuiltProgram);
  byId('reset-input').addEventListener('click', () => {
    byId('input-json').value = state.inputDefaultText;
    putText('input-json-status', state.draft?.inputs?.length ? `${state.draft.inputs.length} DECLARED INPUT${state.draft.inputs.length === 1 ? '' : 'S'}` : 'NO INPUTS DECLARED');
  });
  byId('reset-manifest').addEventListener('click', () => {
    byId('manifest-json').value = state.defaultManifestText;
    updateManifestStatus();
  });
  byId('manifest-json').addEventListener('input', updateManifestStatus);
  byId('input-json').addEventListener('input', () => putText('input-json-status', 'EDITED INPUT'));
  byId('fit-graph').addEventListener('click', () => state.graph?.fit());
  byId('graph-zoom-in').addEventListener('click', () => state.graph?.zoom(1));
  byId('graph-zoom-out').addEventListener('click', () => state.graph?.zoom(-1));
  byId('add-memory-button').addEventListener('click', () => {
    const selected = byId('add-memory-select').value;
    const memory = allMemories().find((entry) => String(entry.digest ?? entry.selector) === selected);
    if (!memory || !state.graph) return;
    state.graph.addMemory(memory);
    byId('add-memory-select').value = '';
  });
  byId('export-draft').addEventListener('click', exportDraft);
  byId('import-draft').addEventListener('click', () => byId('draft-file-input').click());
  byId('draft-file-input').addEventListener('change', importDraft);
  byId('seal-name').addEventListener('input', updateActionAvailability);
  byId('seal-version').addEventListener('input', updateActionAvailability);
  window.addEventListener('resize', () => state.graph?.fit());
}

function exportDraft() {
  const draft = state.graph?.getGraph?.() ?? state.draft;
  if (!draft) {
    setNotice('There is no blueprint draft to export.', 'error');
    return;
  }
  const payload = `${JSON.stringify(draft, null, 2)}\n`;
  const blob = new Blob([payload], { type: 'application/json' });
  const url = URL.createObjectURL(blob);
  const anchor = node('a');
  anchor.href = url;
  const title = getProgramTitle(activeProgram()).toLowerCase().replace(/[^a-z0-9]+/g, '-').replace(/^-|-$/g, '') || 'blueprint';
  anchor.download = `${title}.compose.json`;
  document.body.append(anchor);
  anchor.click();
  anchor.remove();
  URL.revokeObjectURL(url);
  setNotice('Blueprint JSON exported.', 'success');
}

async function importDraft(event) {
  const file = event.target.files?.[0];
  event.target.value = '';
  if (!file) return;
  try {
    const draft = JSON.parse(await file.text());
    if (!draft || typeof draft !== 'object' || draft.schema !== 'nexa.blueprint-draft/0.3'
      || !Array.isArray(draft.nodes) || !Array.isArray(draft.connections)) {
      throw new Error('Choose a Nexa blueprint draft document (nexa.blueprint-draft/0.3).');
    }
    const example = programForEntry(draft.entry);
    const filenameTitle = file.name.replace(/\.json$/i, '').replace(/[._-]+/g, ' ').trim();
    const customId = `import:${draft.entry ?? 'blueprint'}:${file.name}`;
    state.customProgram = {
      id: customId,
      title: example ? `${example.title} · imported draft` : (filenameTitle || draft.entry || 'Imported blueprint'),
      description: `Imported from ${file.name}.`,
      draft,
      inputs: example?.inputs ?? null,
      manifest: copyJson(example?.manifest ?? emptyApplicationManifest()),
    };
    state.programId = customId;
    state.activeManifest = state.customProgram.manifest;
    state.defaultManifestText = JSON.stringify(state.activeManifest, null, 2);
    state.draft = draft;
    state.draftRevision += 1;
    state.builtProgram = null;
    state.builtSubject = null;
    state.inspection = null;
    state.builtRevision = null;
    state.inputDefaultText = inputForDraft(draft, state.customProgram.inputs);
    byId('input-json').value = state.inputDefaultText;
    byId('manifest-json').value = state.defaultManifestText;
    renderInputContract(draft.inputs);
    putText('input-json-status', draft.inputs?.length ? `${draft.inputs.length} DECLARED INPUT${draft.inputs.length === 1 ? '' : 'S'}` : 'NO INPUTS DECLARED');
    putText('manifest-json-status', 'PROGRAM MANIFEST');
    if (state.graph) {
      state.suppressGraphChange = true;
      state.graph.setGraph(copyJson(draft), allMemories());
      state.suppressGraphChange = false;
      state.graph.fit();
    }
    updateProgramSelectors();
    putText('graph-program-name', getProgramTitle(state.customProgram));
    saveDraftLocally();
    clearVisibleBuildState();
    renderGrantOptions();
    renderSelection(null);
    putText('playground-state-label', 'READY WHEN BUILT');
    putText('graph-revision', 'IMPORTED · UNBUILT');
    setNotice(`Imported ${file.name} · build it locally to check the contracts.`, 'success');
  } catch (error) {
    setNotice(error.message || 'The selected file is not valid blueprint JSON.', 'error');
  }
}

async function bootstrap() {
  bindEvents();
  loadGraphEditor();
  const initialView = location.hash.slice(1);
  switchView(['overview', 'memories', 'blueprints', 'playground', 'runs'].includes(initialView) ? initialView : 'blueprints', { updateHash: false });
  await refreshWorkspace();
  if (state.workspace) {
    state.selectedMemoryDigest = allMemories()[0]?.digest ?? allMemories()[0]?.selector ?? null;
    updateProgramSelectors();
    if (state.programId == null && allPrograms()[0]) setActiveProgram(allPrograms()[0].id);
    if (state.graph && state.draft) {
      state.suppressGraphChange = true;
      state.graph.setGraph(copyJson(state.draft), allMemories());
      state.suppressGraphChange = false;
      state.graph.fit();
    }
    renderOverview();
    renderMemoryCatalog();
  }
}

bootstrap();
