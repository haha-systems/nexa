const SVG_NS = 'http://www.w3.org/2000/svg';
const NODE_WIDTH = 252;
const COLUMN_STEP = 358;
const CARD_GAP = 52;

function clone(value) {
  return value == null ? value : JSON.parse(JSON.stringify(value));
}

function endpointKey(endpoint) {
  return JSON.stringify([endpoint.kind, endpoint.node ?? null, endpoint.port]);
}

function safeIdPart(value) {
  const result = String(value ?? 'memory')
    .replace(/[^A-Za-z0-9_]+/g, '_')
    .replace(/^([^A-Za-z_])/, '_$1')
    .replace(/^_+|_+$/g, '') || 'memory';
  return result;
}

function makeElement(tag, className, text) {
  const element = document.createElement(tag);
  if (className) element.className = className;
  if (text != null) element.textContent = text;
  return element;
}

function getExports(memory) {
  return Array.isArray(memory?.exports) ? memory.exports : [];
}

function getSelector(memory) {
  return memory?.selector || memory?.name || '';
}

function portTypeName(type) {
  return type || 'Unknown';
}

function portTypeClass(type) {
  const value = String(type || '').toLowerCase();
  if (value === 'string') return 'type-string';
  if (value === 'unit') return 'type-unit';
  return value ? 'type-other' : 'type-unknown';
}

function displayPortType(type) {
  const value = String(portTypeName(type));
  const digestMarker = value.lastIndexOf('sha256:');
  if (digestMarker >= 0) {
    const digestStart = digestMarker + 'sha256:'.length;
    const digest = value.slice(digestStart).replace(/[)>\]}\s]+$/, '');
    if (digest.length > 20) {
      const shortDigest = `${digest.slice(0, 8)}…${digest.slice(-6)}`;
      return `${value.slice(0, digestStart)}${shortDigest}${value.slice(digestStart + digest.length)}`;
    }
  }
  return value.length > 32 ? `${value.slice(0, 16)}…${value.slice(-10)}` : value;
}

/**
 * Interactive editor for the semantic BlueprintDraft graph. Canvas placement,
 * selection, zoom, and pan are kept in this instance and never enter the draft.
 */
export class GraphEditor {
  constructor(container, { onChange = () => {}, onSelect = () => {} } = {}) {
    if (!container) throw new TypeError('GraphEditor needs a container element.');
    this.container = container;
    this.onChange = onChange;
    this.onSelect = onSelect;
    this.draft = null;
    this.memories = [];
    this.positions = new Map();
    this.selection = null;
    this.pendingSource = null;
    this.gesture = null;
    this.scale = 1;
    this.tx = 0;
    this.ty = 0;
    this.viewManuallyChanged = false;
    this.suppressPortClick = false;
    this.portDots = new Map();
    this.nodeElements = new Map();
    this.endpointTypes = new Map();
    this.portDescriptors = new Map();
    this.connectionIssues = [];
    this.destroyed = false;

    this.root = makeElement('section', 'nx-graph');
    this.root.setAttribute('aria-label', 'Blueprint graph editor');
    this.root.tabIndex = 0;

    this.header = makeElement('header', 'nx-graph-header');
    const titleBlock = makeElement('div', 'nx-graph-title');
    const eyebrow = makeElement('span', 'nx-graph-eyebrow', 'BLUEPRINT / VISUAL EDITOR');
    this.title = makeElement('strong', 'nx-graph-title-name', 'Composition graph');
    titleBlock.append(eyebrow, this.title);

    this.status = makeElement('div', 'nx-graph-status');
    this.status.setAttribute('role', 'status');
    this.status.setAttribute('aria-live', 'polite');
    this.statusText = makeElement('span', 'nx-graph-status-text', 'Waiting for a blueprint draft');
    this.status.append(makeElement('i', 'nx-graph-status-dot'), this.statusText);

    const controls = makeElement('div', 'nx-graph-controls');
    this.zoomReadout = makeElement('span', 'nx-graph-zoom-readout', '100%');
    this.deleteButton = this.makeControl('DELETE', 'Delete selected node or connection', () => this.deleteSelection(), 'nx-graph-delete-button');
    this.deleteButton.disabled = true;
    controls.append(this.zoomReadout, this.deleteButton);
    this.header.append(titleBlock, this.status, controls);

    this.viewport = makeElement('div', 'nx-graph-viewport');
    this.viewport.setAttribute('role', 'application');
    this.viewport.setAttribute('aria-label', 'Blueprint canvas. Drag nodes to arrange them, or drag the background to pan.');
    this.viewport.tabIndex = 0;
    this.world = makeElement('div', 'nx-graph-world');
    this.svg = document.createElementNS(SVG_NS, 'svg');
    this.svg.classList.add('nx-graph-wires');
    this.nodeLayer = makeElement('div', 'nx-graph-nodes');
    this.world.append(this.svg, this.nodeLayer);
    this.viewport.append(this.world);

    this.footer = makeElement('footer', 'nx-graph-footer');
    this.hint = makeElement('span', 'nx-graph-hint', 'DRAG NODES TO ARRANGE');
    const footerHelp = makeElement('span', 'nx-graph-help', 'Drag from an output port to an input, or click the ports in sequence. DELETE removes a selected node or wire.');
    this.footer.append(this.hint, footerHelp);

    this.root.append(this.header, this.viewport, this.footer);
    this.container.replaceChildren(this.root);

    this.handlePointerDown = this.handlePointerDown.bind(this);
    this.handlePointerMove = this.handlePointerMove.bind(this);
    this.handlePointerUp = this.handlePointerUp.bind(this);
    this.handleClick = this.handleClick.bind(this);
    this.handleKeyDown = this.handleKeyDown.bind(this);
    this.handleWheel = this.handleWheel.bind(this);
    this.viewport.addEventListener('pointerdown', this.handlePointerDown);
    this.viewport.addEventListener('click', this.handleClick);
    this.viewport.addEventListener('wheel', this.handleWheel, { passive: false });
    this.root.addEventListener('keydown', this.handleKeyDown);
    document.addEventListener('pointermove', this.handlePointerMove);
    document.addEventListener('pointerup', this.handlePointerUp);

    this.resizeObserver = typeof ResizeObserver === 'function'
      ? new ResizeObserver(() => {
        if (!this.viewManuallyChanged) this.fit();
      })
      : null;
    this.resizeObserver?.observe(this.viewport);

    this.setGraph({ nodes: [], connections: [], inputs: [], output: null, order_edges: [] }, []);
  }

  makeControl(label, ariaLabel, action, className) {
    const button = makeElement('button', className, label);
    button.type = 'button';
    button.setAttribute('aria-label', ariaLabel);
    button.addEventListener('click', action);
    return button;
  }

  setGraph(draft, memories = []) {
    if (this.destroyed) return;
    this.draft = clone(draft || { nodes: [], connections: [], inputs: [], output: null, order_edges: [] });
    this.draft.nodes ??= [];
    this.draft.connections ??= [];
    this.draft.inputs ??= [];
    this.draft.order_edges ??= [];
    this.memories = Array.isArray(memories) ? [...memories] : [];
    this.notice = null;
    this.title.textContent = this.draft.entry || 'Composition graph';

    const existingIds = new Set(this.draft.nodes.map((node) => node.id));
    for (const id of this.positions.keys()) if (!existingIds.has(id)) this.positions.delete(id);
    this.placeMissingNodes();
    this.render();
    requestAnimationFrame(() => {
      if (!this.viewManuallyChanged) this.fit();
    });
  }

  getGraph() {
    return clone(this.draft);
  }

  /** Add a memory as a draft node, choosing its first callable export. */
  addMemory(memory) {
    if (!this.draft || !memory) return false;
    const selector = getSelector(memory);
    const callable = getExports(memory)[0];
    if (!selector || !callable?.export_name) {
      this.setNotice('This memory has no selectable callable export.');
      return false;
    }
    const seed = safeIdPart(callable.export_name || memory.name || 'memory');
    const used = new Set(this.draft.nodes.map((node) => node.id));
    let id = seed;
    let suffix = 2;
    while (used.has(id)) id = `${seed}_${suffix++}`;
    this.draft.nodes.push({ id, selector, export: callable.export_name });
    if (!this.memories.some((item) => getSelector(item) === selector)) this.memories.push(memory);
    this.positions.delete(id);
    this.notice = null;
    this.placeMissingNodes();
    this.selection = { kind: 'node', id };
    this.pendingSource = null;
    this.render();
    this.emitChange();
    this.selectNode(id);
    this.fit();
    return true;
  }

  fit() {
    if (this.destroyed || !this.viewport || !this.draft) return;
    const viewportWidth = Math.max(1, this.viewport.clientWidth);
    const viewportHeight = Math.max(1, this.viewport.clientHeight);
    const box = this.worldBounds();
    const margin = 48;
    const factor = Math.min(
      (viewportWidth - margin * 2) / Math.max(1, box.width),
      (viewportHeight - margin * 2) / Math.max(1, box.height),
      1,
    );
    this.scale = Math.max(0.38, Math.min(1.2, factor));
    this.tx = (viewportWidth - box.width * this.scale) / 2 - box.minX * this.scale;
    this.ty = (viewportHeight - box.height * this.scale) / 2 - box.minY * this.scale;
    this.viewManuallyChanged = false;
    this.applyView();
  }

  /** Zoom one or more steps. Positive values zoom in; negative values zoom out. */
  zoom(delta) {
    if (!Number.isFinite(Number(delta)) || Number(delta) === 0) return;
    const bounds = this.viewport.getBoundingClientRect();
    this.zoomAt(Math.pow(1.14, Number(delta)), bounds.left + bounds.width / 2, bounds.top + bounds.height / 2);
  }

  destroy() {
    if (this.destroyed) return;
    this.destroyed = true;
    this.resizeObserver?.disconnect();
    this.viewport.removeEventListener('pointerdown', this.handlePointerDown);
    this.viewport.removeEventListener('click', this.handleClick);
    this.viewport.removeEventListener('wheel', this.handleWheel);
    this.root.removeEventListener('keydown', this.handleKeyDown);
    document.removeEventListener('pointermove', this.handlePointerMove);
    document.removeEventListener('pointerup', this.handlePointerUp);
    if (this.container.contains(this.root)) this.container.removeChild(this.root);
  }

  memoryFor(node) {
    return this.memories.find((memory) => (
      getSelector(memory) === node.selector || memory?.digest === node.selector
    )) || null;
  }

  contractFor(node) {
    const memory = this.memoryFor(node);
    return getExports(memory).find((contract) => contract.export_name === node.export) || null;
  }

  portLists(node) {
    const contract = this.contractFor(node);
    const knownInputs = (contract?.parameters || []).map((port) => ({ ...port, known: true }));
    const names = new Set(knownInputs.map((port) => port.name));
    const referencedInputs = this.draft.connections
      .filter((connection) => connection.to?.kind === 'node' && connection.to.node === node.id)
      .map((connection) => connection.to.port);
    for (const name of referencedInputs) {
      if (!names.has(name)) knownInputs.push({ name, type: null, known: false });
    }
    const resultType = contract?.result_type || null;
    return {
      contract,
      inputs: knownInputs,
      output: { name: 'result', type: resultType, known: Boolean(contract) },
    };
  }

  nodeHeight(node) {
    const { inputs } = this.portLists(node);
    return Math.max(136, 92 + Math.max(inputs.length, 1) * 27);
  }

  computeRanks() {
    const nodes = this.draft.nodes;
    const ids = new Set(nodes.map((node) => node.id));
    const incomingCount = new Map(nodes.map((node) => [node.id, 0]));
    const outgoing = new Map(nodes.map((node) => [node.id, []]));
    for (const connection of this.draft.connections) {
      const from = connection.from;
      const to = connection.to;
      if (from?.kind === 'node' && to?.kind === 'node' && ids.has(from.node) && ids.has(to.node)) {
        outgoing.get(from.node).push(to.node);
        incomingCount.set(to.node, incomingCount.get(to.node) + 1);
      }
    }
    const rank = new Map(nodes.map((node) => [node.id, 0]));
    const ready = nodes.filter((node) => incomingCount.get(node.id) === 0).map((node) => node.id);
    const visited = new Set();
    while (ready.length) {
      const id = ready.shift();
      if (visited.has(id)) continue;
      visited.add(id);
      for (const next of outgoing.get(id)) {
        rank.set(next, Math.max(rank.get(next), rank.get(id) + 1));
        incomingCount.set(next, incomingCount.get(next) - 1);
        if (incomingCount.get(next) === 0) ready.push(next);
      }
    }
    // Cycles remain visible as draft structure; place their nodes in a column
    // without attempting to validate or repair them here.
    for (const node of nodes) if (!visited.has(node.id)) rank.set(node.id, 0);
    return rank;
  }

  placeMissingNodes() {
    if (!this.draft?.nodes?.length) return;
    const missing = this.draft.nodes.filter((node) => !this.positions.has(node.id));
    if (!missing.length) return;
    const ranks = this.computeRanks();
    if (!this.positions.size) {
      const byRank = new Map();
      for (const node of this.draft.nodes) {
        const rank = ranks.get(node.id) || 0;
        if (!byRank.has(rank)) byRank.set(rank, []);
        byRank.get(rank).push(node);
      }
      for (const [rank, group] of byRank) {
        const height = group.reduce((total, node) => total + this.nodeHeight(node) + CARD_GAP, 0) - CARD_GAP;
        let y = 150 + Math.max(0, (680 - height) / 2);
        for (const node of group) {
          this.positions.set(node.id, { x: 340 + rank * COLUMN_STEP, y });
          y += this.nodeHeight(node) + CARD_GAP;
        }
      }
      return;
    }
    for (const node of missing) {
      const rank = ranks.get(node.id) || 0;
      const x = 340 + rank * COLUMN_STEP;
      const peers = this.draft.nodes
        .filter((other) => other.id !== node.id && this.positions.has(other.id) && Math.abs(this.positions.get(other.id).x - x) < COLUMN_STEP / 2)
        .map((other) => this.positions.get(other.id).y + this.nodeHeight(other));
      const y = peers.length ? Math.max(...peers) + CARD_GAP : 150;
      this.positions.set(node.id, { x, y });
    }
  }

  worldBounds() {
    let minX = 0;
    let minY = 0;
    let maxX = 1000;
    let maxY = 760;
    for (const node of this.draft.nodes) {
      const position = this.positions.get(node.id) || { x: 340, y: 150 };
      minX = Math.min(minX, position.x - 48);
      minY = Math.min(minY, position.y - 48);
      maxX = Math.max(maxX, position.x + NODE_WIDTH + 48);
      maxY = Math.max(maxY, position.y + this.nodeHeight(node) + 48);
    }
    // Terminal cards sit just outside the node columns.
    maxX += this.draft.nodes.length ? 240 : 0;
    return { minX, minY, maxX, maxY, width: maxX - minX, height: maxY - minY };
  }

  render() {
    this.portDots.clear();
    this.nodeElements.clear();
    this.endpointTypes.clear();
    this.portDescriptors.clear();
    this.nodeLayer.replaceChildren();
    this.svg.replaceChildren();

    const ranks = this.computeRanks();
    const maxRank = this.draft.nodes.reduce((value, node) => Math.max(value, ranks.get(node.id) || 0), 0);
    const maxColumnHeight = Math.max(0, ...Array.from({ length: maxRank + 1 }, (_, rank) => {
      const group = this.draft.nodes.filter((node) => (ranks.get(node.id) || 0) === rank);
      return group.reduce((total, node) => total + this.nodeHeight(node) + CARD_GAP, 0) - (group.length ? CARD_GAP : 0);
    }));
    const inputHeight = 98 + this.draft.inputs.length * 28;
    const worldHeight = Math.max(760, maxColumnHeight + 280, inputHeight + 260);
    const outputX = this.draft.nodes.length ? 340 + maxRank * COLUMN_STEP + 330 : 700;
    const worldWidth = Math.max(1080, outputX + 290);
    this.world.style.width = `${worldWidth}px`;
    this.world.style.height = `${worldHeight}px`;
    this.svg.setAttribute('viewBox', `0 0 ${worldWidth} ${worldHeight}`);
    this.svg.setAttribute('width', String(worldWidth));
    this.svg.setAttribute('height', String(worldHeight));

    this.renderInputTerminal(Math.max(150, worldHeight / 2 - inputHeight / 2));
    const byRank = new Map();
    for (const node of this.draft.nodes) {
      const rank = ranks.get(node.id) || 0;
      if (!byRank.has(rank)) byRank.set(rank, []);
      byRank.get(rank).push(node);
    }
    for (const [rank, group] of byRank) {
      if (this.positions.size && group.every((node) => this.positions.has(node.id))) {
        // Keep the user's positions once established.
        for (const node of group) this.renderNode(node);
        continue;
      }
      for (const node of group) this.renderNode(node);
    }
    this.renderOutputTerminal(outputX, Math.max(150, worldHeight / 2 - 76));
    this.renderConnections();
    this.applyView();
    this.updateStatus();
    this.updateSelectionVisuals();
  }

  renderInputTerminal(y) {
    const card = makeElement('article', 'nx-terminal-card nx-input-terminal');
    card.style.left = '42px';
    card.style.top = `${y}px`;
    const title = makeElement('div', 'nx-terminal-head');
    title.append(makeElement('span', 'nx-terminal-symbol', 'IN'), makeElement('strong', '', 'Blueprint inputs'));
    const sub = makeElement('div', 'nx-terminal-sub', `${this.draft.inputs.length} exposed ${this.draft.inputs.length === 1 ? 'value' : 'values'}`);
    const rows = makeElement('div', 'nx-terminal-ports');
    for (const port of this.draft.inputs) {
      const endpoint = { kind: 'input', port: port.name };
      const button = this.makePortButton(port, 'source', 'terminal');
      this.registerPort(endpoint, port, button);
      rows.append(button);
    }
    if (!this.draft.inputs.length) rows.append(makeElement('div', 'nx-terminal-empty', 'No inputs defined'));
    card.append(title, sub, rows);
    this.nodeLayer.append(card);
  }

  renderOutputTerminal(x, y) {
    const card = makeElement('article', 'nx-terminal-card nx-output-terminal');
    card.style.left = `${x}px`;
    card.style.top = `${y}px`;
    const title = makeElement('div', 'nx-terminal-head');
    title.append(makeElement('span', 'nx-terminal-symbol nx-output-symbol', 'OUT'), makeElement('strong', '', 'Blueprint output'));
    const sub = makeElement('div', 'nx-terminal-sub', 'GRAPH TERMINAL');
    const rows = makeElement('div', 'nx-terminal-ports');
    const port = this.draft.output;
    if (port?.name) {
      const endpoint = { kind: 'output', port: port.name };
      const button = this.makePortButton(port, 'target', 'terminal');
      this.registerPort(endpoint, port, button);
      rows.append(button);
    } else {
      rows.append(makeElement('div', 'nx-terminal-empty', 'No output defined'));
    }
    card.append(title, sub, rows);
    this.nodeLayer.append(card);
  }

  renderNode(node) {
    const memory = this.memoryFor(node);
    const { contract, inputs, output } = this.portLists(node);
    const kind = contract?.kind || 'unknown';
    const position = this.positions.get(node.id) || { x: 340, y: 150 };
    const kindClass = kind === 'action' ? 'is-action' : kind === 'function' ? 'is-function' : 'is-unresolved';
    const card = makeElement('article', `nx-node-card ${kindClass}`);
    card.dataset.nodeId = node.id;
    card.style.left = `${position.x}px`;
    card.style.top = `${position.y}px`;
    card.style.width = `${NODE_WIDTH}px`;
    card.setAttribute('aria-label', `${kind} node ${node.id}`);

    const head = makeElement('div', 'nx-node-head');
    const badgeText = kind === 'action' ? 'ACTION' : kind === 'function' ? 'FUNCTION' : 'CONTRACT?';
    const badge = makeElement('span', `nx-node-kind ${kind === 'action' ? 'is-action' : ''}`, badgeText);
    const identity = makeElement('span', 'nx-node-identity', node.id);
    const menu = makeElement('span', 'nx-node-grip', '⠿');
    menu.setAttribute('aria-hidden', 'true');
    head.append(badge, identity, menu);

    const body = makeElement('div', 'nx-node-body');
    body.append(makeElement('strong', 'nx-node-name', memory?.name || node.selector));
    body.append(makeElement('span', 'nx-node-export', `${node.selector}  /  ${node.export}`));
    const portArea = makeElement('div', 'nx-node-port-area');
    const inputColumn = makeElement('div', 'nx-node-inputs');
    inputColumn.append(makeElement('span', 'nx-port-column-label', 'INPUTS'));
    for (const port of inputs) {
      const endpoint = { kind: 'node', node: node.id, port: port.name };
      const button = this.makePortButton(port, 'target', port.known ? 'known' : 'unknown');
      this.registerPort(endpoint, port, button);
      inputColumn.append(button);
    }
    if (!inputs.length) inputColumn.append(makeElement('div', 'nx-no-inputs', 'No inputs'));

    const outputColumn = makeElement('div', 'nx-node-outputs');
    outputColumn.append(makeElement('span', 'nx-port-column-label', 'OUTPUT'));
    const outputEndpoint = { kind: 'node', node: node.id, port: output.name };
    const outputButton = this.makePortButton(output, 'source', output.known ? 'known' : 'unknown');
    this.registerPort(outputEndpoint, output, outputButton);
    outputColumn.append(outputButton);
    portArea.append(inputColumn, outputColumn);

    const foot = makeElement('div', 'nx-node-foot');
    foot.append(makeElement('span', 'nx-node-version', memory?.version ? `MEMORY ${memory.version}` : 'MEMORY CONTRACT'));
    if (!contract) foot.append(makeElement('span', 'nx-node-unresolved', 'UNRESOLVED'));
    card.append(head, body, portArea, foot);
    card.addEventListener('click', (event) => {
      if (this.suppressPortClick) {
        this.suppressPortClick = false;
        return;
      }
      if (event.target.closest('button')) return;
      this.selectNode(node.id);
    });
    this.nodeElements.set(node.id, card);
    this.nodeLayer.append(card);
  }

  makePortButton(port, direction, contractState) {
    const button = makeElement('button', `nx-port nx-port-${direction} ${port.type ? portTypeClass(port.type) : ''} ${contractState === 'unknown' ? 'is-unknown' : ''}`);
    button.type = 'button';
    button.dataset.portDirection = direction;
    button.dataset.portName = port.name;
    const label = makeElement('span', 'nx-port-label', port.name);
    const type = makeElement('span', `nx-port-type ${port.type ? portTypeClass(port.type) : 'type-unknown'}`, displayPortType(port.type));
    type.title = String(portTypeName(port.type));
    const dot = makeElement('span', 'nx-port-dot');
    dot.setAttribute('aria-hidden', 'true');
    if (direction === 'source') button.append(label, type, dot);
    else button.append(dot, label, type);
    button.setAttribute('aria-label', `${direction === 'source' ? 'Output' : 'Input'} port ${port.name}, type ${portTypeName(port.type)}${contractState === 'unknown' ? ', contract unresolved' : ''}`);
    return button;
  }

  registerPort(endpoint, descriptor, button) {
    const key = endpointKey(endpoint);
    this.portDots.set(key, button.querySelector('.nx-port-dot'));
    this.endpointTypes.set(key, descriptor?.type || null);
    this.portDescriptors.set(key, { endpoint, descriptor, button });
    button.dataset.endpointKey = key;
  }

  renderConnections() {
    this.svg.replaceChildren();
    this.connectionIssues = [];
    this.draft.connections.forEach((connection, index) => {
      const fromKey = endpointKey(connection.from || {});
      const toKey = endpointKey(connection.to || {});
      const fromDot = this.portDots.get(fromKey);
      const toDot = this.portDots.get(toKey);
      if (!fromDot || !toDot) {
        this.connectionIssues.push({ index, kind: 'unresolved' });
        return;
      }
      const fromType = this.endpointTypes.get(fromKey);
      const toType = this.endpointTypes.get(toKey);
      const mismatch = Boolean(fromType && toType && fromType !== toType);
      if (mismatch) this.connectionIssues.push({ index, kind: 'mismatch', fromType, toType });
      const { start, end } = this.portCoordinates(fromDot, toDot);
      const pull = Math.max(64, Math.min(190, Math.abs(end.x - start.x) * 0.46));
      const pathData = `M ${start.x} ${start.y} C ${start.x + pull} ${start.y}, ${end.x - pull} ${end.y}, ${end.x} ${end.y}`;
      const selected = this.selection?.kind === 'connection' && this.selection.index === index;
      const valueClass = fromType === 'Unit' ? 'type-unit' : fromType === 'String' ? 'type-string' : 'type-unknown';
      const wireClass = `nx-wire ${valueClass} ${mismatch ? 'is-mismatch' : ''} ${selected ? 'is-selected' : ''}`;
      const visible = document.createElementNS(SVG_NS, 'path');
      visible.setAttribute('d', pathData);
      visible.setAttribute('class', `nx-wire-visible ${valueClass} ${mismatch ? 'is-mismatch' : ''} ${selected ? 'is-selected' : ''}`);
      visible.setAttribute('fill', 'none');
      visible.setAttribute('vector-effect', 'non-scaling-stroke');
      visible.setAttribute('stroke-width', selected ? '3.5' : '2.5');
      visible.setAttribute('pointer-events', 'none');
      this.svg.append(visible);

      const hit = document.createElementNS(SVG_NS, 'path');
      hit.setAttribute('d', pathData);
      hit.setAttribute('class', wireClass);
      hit.setAttribute('fill', 'none');
      hit.setAttribute('stroke', 'transparent');
      hit.setAttribute('stroke-width', '18');
      hit.setAttribute('tabindex', '0');
      hit.setAttribute('role', 'button');
      hit.setAttribute('aria-label', `Connection ${index + 1}: ${this.describeEndpoint(connection.from)} to ${this.describeEndpoint(connection.to)}${mismatch ? ', type mismatch' : ''}`);
      hit.dataset.connectionIndex = String(index);
      hit.addEventListener('click', (event) => {
        event.stopPropagation();
        this.selectConnection(index);
      });
      hit.addEventListener('keydown', (event) => {
        if (event.key === 'Enter' || event.key === ' ') {
          event.preventDefault();
          this.selectConnection(index);
        }
      });
      this.svg.append(hit);
    });
  }

  portCoordinates(fromDot, toDot) {
    const worldRect = this.world.getBoundingClientRect();
    const fromRect = fromDot.getBoundingClientRect();
    const toRect = toDot.getBoundingClientRect();
    return {
      start: {
        x: (fromRect.left + fromRect.width / 2 - worldRect.left) / this.scale,
        y: (fromRect.top + fromRect.height / 2 - worldRect.top) / this.scale,
      },
      end: {
        x: (toRect.left + toRect.width / 2 - worldRect.left) / this.scale,
        y: (toRect.top + toRect.height / 2 - worldRect.top) / this.scale,
      },
    };
  }

  describeEndpoint(endpoint) {
    if (!endpoint) return 'unknown port';
    if (endpoint.kind === 'input') return `input.${endpoint.port}`;
    if (endpoint.kind === 'output') return `output.${endpoint.port}`;
    return `${endpoint.node}.${endpoint.port}`;
  }

  selectNode(id) {
    const node = this.draft.nodes.find((item) => item.id === id);
    if (!node) return;
    this.pendingSource = null;
    this.selection = { kind: 'node', id };
    this.renderConnections();
    this.updateSelectionVisuals();
    this.onSelect({ kind: 'node', id, node: clone(node), memory: this.memoryFor(node) });
  }

  selectConnection(index) {
    const connection = this.draft.connections[index];
    if (!connection) return;
    this.pendingSource = null;
    this.selection = { kind: 'connection', index };
    this.renderConnections();
    this.updateSelectionVisuals();
    this.onSelect({ kind: 'connection', id: String(index), connection: clone(connection) });
  }

  selectPort(endpoint) {
    const key = endpointKey(endpoint);
    const info = this.portDescriptors.get(key);
    if (!info) return;
    const ownerNode = endpoint.kind === 'node' ? this.draft.nodes.find((node) => node.id === endpoint.node) : null;
    let kind;
    if (endpoint.kind === 'input') kind = 'input';
    else if (endpoint.kind === 'output') kind = 'output';
    else kind = info.button.dataset.portDirection === 'source' ? 'output' : 'input';
    this.selection = { kind: 'port', endpoint };
    this.updateSelectionVisuals();
    this.onSelect({
      kind,
      id: endpoint.port,
      node: ownerNode ? clone(ownerNode) : undefined,
      memory: ownerNode ? this.memoryFor(ownerNode) : undefined,
      port: endpoint.port,
      type: info.descriptor?.type || undefined,
      portInfo: clone(info.descriptor),
    });
  }

  handleClick(event) {
    const portButton = event.target.closest('.nx-port');
    if (!portButton) {
      if (event.target === this.viewport || event.target === this.world || event.target === this.svg) {
        this.pendingSource = null;
        this.selection = null;
        this.updateSelectionVisuals();
        this.renderConnections();
        this.onSelect(null);
      }
      return;
    }
    if (this.suppressPortClick) {
      this.suppressPortClick = false;
      return;
    }
    const info = this.portDescriptors.get(portButton.dataset.endpointKey);
    if (!info) return;
    const direction = portButton.dataset.portDirection;
    if (direction === 'source') {
      this.pendingSource = clone(info.endpoint);
      this.selectPort(info.endpoint);
      return;
    }
    if (this.pendingSource) {
      this.connect(this.pendingSource, info.endpoint);
      return;
    }
    this.selectPort(info.endpoint);
  }

  handlePointerDown(event) {
    if (event.button !== 0 || this.destroyed) return;
    const port = event.target.closest('.nx-port');
    if (port) {
      if (port.dataset.portDirection === 'source') {
        const info = this.portDescriptors.get(port.dataset.endpointKey);
        if (info) this.gesture = {
          type: 'wire', pointerId: event.pointerId, source: clone(info.endpoint),
          startX: event.clientX, startY: event.clientY, moved: false, clientX: event.clientX, clientY: event.clientY,
        };
      }
      return;
    }
    const nodeHead = event.target.closest('.nx-node-head');
    if (nodeHead) {
      const card = nodeHead.closest('.nx-node-card');
      if (!card) return;
      event.preventDefault();
      const id = card.dataset.nodeId;
      this.selectNode(id);
      const position = this.positions.get(id);
      this.gesture = {
        type: 'node', pointerId: event.pointerId, nodeId: id,
        startX: event.clientX, startY: event.clientY,
        x: position.x, y: position.y, moved: false,
      };
      return;
    }
    if (event.target.closest('.nx-wire')) return;
    if (event.target.closest('.nx-node-card')) return;
    this.gesture = {
      type: 'pan', pointerId: event.pointerId,
      startX: event.clientX, startY: event.clientY,
      tx: this.tx, ty: this.ty, moved: false,
    };
    this.root.classList.add('is-panning');
  }

  handlePointerMove(event) {
    const gesture = this.gesture;
    if (!gesture || gesture.pointerId !== event.pointerId || this.destroyed) return;
    const dx = event.clientX - gesture.startX;
    const dy = event.clientY - gesture.startY;
    if (Math.abs(dx) + Math.abs(dy) > 4) gesture.moved = true;
    if (gesture.type === 'node') {
      if (!gesture.moved) return;
      const position = { x: gesture.x + dx / this.scale, y: gesture.y + dy / this.scale };
      this.positions.set(gesture.nodeId, position);
      const card = this.nodeElements.get(gesture.nodeId);
      if (card) {
        card.style.left = `${position.x}px`;
        card.style.top = `${position.y}px`;
      }
      this.viewManuallyChanged = true;
      this.renderConnections();
    } else if (gesture.type === 'pan') {
      this.tx = gesture.tx + dx;
      this.ty = gesture.ty + dy;
      this.viewManuallyChanged = true;
      this.applyView();
    } else if (gesture.type === 'wire') {
      gesture.clientX = event.clientX;
      gesture.clientY = event.clientY;
      if (gesture.moved) this.renderPreviewWire(gesture.source, event.clientX, event.clientY);
    }
  }

  handlePointerUp(event) {
    const gesture = this.gesture;
    if (!gesture || gesture.pointerId !== event.pointerId || this.destroyed) return;
    if (gesture.type === 'wire' && gesture.moved) {
      const element = document.elementFromPoint(event.clientX, event.clientY);
      const targetButton = element?.closest('.nx-port[data-port-direction="target"]');
      const targetInfo = targetButton && this.portDescriptors.get(targetButton.dataset.endpointKey);
      if (targetInfo) {
        this.connect(gesture.source, targetInfo.endpoint);
        this.suppressPortClick = true;
        setTimeout(() => { this.suppressPortClick = false; }, 0);
      }
      this.svg.querySelector('.nx-wire-preview')?.remove();
    }
    if (gesture.type === 'node' && gesture.moved) {
      this.suppressPortClick = true;
      setTimeout(() => { this.suppressPortClick = false; }, 0);
    }
    if (gesture.type === 'pan') this.root.classList.remove('is-panning');
    this.gesture = null;
    this.renderConnections();
  }

  renderPreviewWire(source, clientX, clientY) {
    const sourceDot = this.portDots.get(endpointKey(source));
    if (!sourceDot) return;
    const worldRect = this.world.getBoundingClientRect();
    const dotRect = sourceDot.getBoundingClientRect();
    const start = {
      x: (dotRect.left + dotRect.width / 2 - worldRect.left) / this.scale,
      y: (dotRect.top + dotRect.height / 2 - worldRect.top) / this.scale,
    };
    const end = { x: (clientX - worldRect.left) / this.scale, y: (clientY - worldRect.top) / this.scale };
    const pull = Math.max(64, Math.min(190, Math.abs(end.x - start.x) * 0.46));
    let path = this.svg.querySelector('.nx-wire-preview');
    if (!path) {
      path = document.createElementNS(SVG_NS, 'path');
      path.setAttribute('class', 'nx-wire-preview');
      path.setAttribute('fill', 'none');
      this.svg.append(path);
    }
    path.setAttribute('d', `M ${start.x} ${start.y} C ${start.x + pull} ${start.y}, ${end.x - pull} ${end.y}, ${end.x} ${end.y}`);
  }

  connect(from, to) {
    const fromInfo = this.portDescriptors.get(endpointKey(from));
    const toInfo = this.portDescriptors.get(endpointKey(to));
    if (!fromInfo || !toInfo) return;
    if (fromInfo.button.dataset.portDirection !== 'source' || toInfo.button.dataset.portDirection !== 'target') return;
    const duplicate = this.draft.connections.findIndex((connection) => endpointKey(connection.from) === endpointKey(from) && endpointKey(connection.to) === endpointKey(to));
    if (duplicate !== -1) {
      this.selectConnection(duplicate);
      this.pendingSource = null;
      return;
    }
    // An input has one incoming wire in this editor; replace its previous wire.
    this.draft.connections = this.draft.connections.filter((connection) => endpointKey(connection.to) !== endpointKey(to));
    this.draft.connections.push({ from: clone(from), to: clone(to) });
    this.pendingSource = null;
    this.selection = { kind: 'connection', index: this.draft.connections.length - 1 };
    this.render();
    const fromType = this.endpointTypes.get(endpointKey(from));
    const toType = this.endpointTypes.get(endpointKey(to));
    if (fromType && toType && fromType !== toType) {
      this.setNotice(`TYPE MISMATCH  /  ${fromType} → ${toType}  /  wire retained in draft`);
    } else {
      this.setNotice(`CONNECTED  /  ${portTypeName(fromType)} → ${portTypeName(toType)}`);
    }
    this.emitChange();
    const index = this.draft.connections.length - 1;
    const connection = this.draft.connections[index];
    this.onSelect({ kind: 'connection', id: String(index), connection: clone(connection) });
    this.updateSelectionVisuals();
  }

  emitChange() {
    this.onChange(this.getGraph());
  }

  setNotice(message) {
    this.notice = message;
    this.updateStatus();
  }

  updateStatus() {
    if (!this.draft) return;
    const mismatches = this.connectionIssues.filter((issue) => issue.kind === 'mismatch').length;
    const unresolved = this.connectionIssues.filter((issue) => issue.kind === 'unresolved').length;
    this.status.classList.toggle('has-issue', mismatches > 0 || unresolved > 0);
    this.status.classList.toggle('has-mismatch', mismatches > 0);
    this.statusText.textContent = mismatches
      ? `TYPE MISMATCH · ${this.connectionIssues.find((issue) => issue.kind === 'mismatch').fromType} → ${this.connectionIssues.find((issue) => issue.kind === 'mismatch').toType} · ${mismatches} ${mismatches === 1 ? 'WIRE' : 'WIRES'}`
      : unresolved
        ? `UNRESOLVED ENDPOINT · ${unresolved}`
        : this.notice || `DRAFT · ${this.draft.nodes.length} ${this.draft.nodes.length === 1 ? 'NODE' : 'NODES'} · ${this.draft.connections.length} WIRES`;
  }

  updateSelectionVisuals() {
    for (const [id, element] of this.nodeElements) {
      element.classList.toggle('is-selected', this.selection?.kind === 'node' && this.selection.id === id);
    }
    for (const [key, button] of this.portDescriptors.entries()) {
      button.button.classList.toggle('is-pending', this.pendingSource && endpointKey(this.pendingSource) === key);
      button.button.classList.toggle('is-selected', this.selection?.kind === 'port' && endpointKey(this.selection.endpoint) === key);
    }
    this.deleteButton.disabled = !['node', 'connection'].includes(this.selection?.kind);
  }

  deleteSelection() {
    if (!this.selection) return false;
    if (this.selection.kind === 'connection') {
      this.draft.connections.splice(this.selection.index, 1);
    } else if (this.selection.kind === 'node') {
      const id = this.selection.id;
      this.draft.nodes = this.draft.nodes.filter((node) => node.id !== id);
      this.draft.connections = this.draft.connections.filter((connection) => connection.from?.node !== id && connection.to?.node !== id);
      this.draft.order_edges = this.draft.order_edges.filter((edge) => edge.before !== id && edge.after !== id);
      this.positions.delete(id);
    } else {
      return false;
    }
    this.selection = null;
    this.pendingSource = null;
    this.notice = null;
    this.render();
    this.onSelect(null);
    this.emitChange();
    return true;
  }

  handleKeyDown(event) {
    if (this.destroyed || event.defaultPrevented || event.altKey || event.ctrlKey || event.metaKey) return;
    const target = event.target;
    if (target?.matches('input,textarea,select,[contenteditable="true"]')) return;
    if (event.key === 'Delete' || event.key === 'Backspace') {
      if (this.deleteSelection()) event.preventDefault();
      return;
    }
    const directions = { ArrowUp: [0, -1], ArrowDown: [0, 1], ArrowLeft: [-1, 0], ArrowRight: [1, 0] };
    const direction = directions[event.key];
    if (!direction || this.selection?.kind !== 'node') return;
    const position = this.positions.get(this.selection.id);
    if (!position) return;
    event.preventDefault();
    const step = event.shiftKey ? 80 : 20;
    const next = { x: position.x + direction[0] * step, y: position.y + direction[1] * step };
    this.positions.set(this.selection.id, next);
    const card = this.nodeElements.get(this.selection.id);
    if (card) {
      card.style.left = `${next.x}px`;
      card.style.top = `${next.y}px`;
    }
    this.viewManuallyChanged = true;
    this.renderConnections();
  }

  handleWheel(event) {
    event.preventDefault();
    const factor = event.deltaY < 0 ? 1.09 : 1 / 1.09;
    this.zoomAt(factor, event.clientX, event.clientY);
  }

  zoomAt(factor, clientX, clientY) {
    const bounds = this.viewport.getBoundingClientRect();
    const px = clientX - bounds.left;
    const py = clientY - bounds.top;
    const nextScale = Math.max(0.38, Math.min(1.6, this.scale * factor));
    const worldX = (px - this.tx) / this.scale;
    const worldY = (py - this.ty) / this.scale;
    this.scale = nextScale;
    this.tx = px - worldX * this.scale;
    this.ty = py - worldY * this.scale;
    this.viewManuallyChanged = true;
    this.applyView();
    this.renderConnections();
  }

  applyView() {
    if (!this.world) return;
    this.world.style.transform = `translate(${this.tx}px, ${this.ty}px) scale(${this.scale})`;
    this.zoomReadout.textContent = `${Math.round(this.scale * 100)}%`;
  }
}

export default GraphEditor;
