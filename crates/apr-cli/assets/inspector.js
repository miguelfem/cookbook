'use strict';
(() => {
  const token = new URLSearchParams(location.hash.slice(1)).get('session');
  // Keep the session only in memory; ordinary navigation never reloads the page.
  history.replaceState(null, '', location.pathname + (token ? '#home' : location.hash || '#home'));
  const $ = id => document.getElementById(id);
  const pages = [...document.querySelectorAll('[data-page]')];
  const pageInfo = {
    home: ['Your cookbook', 'Recipes, runs, and the work that needs your attention.'],
    recipes: ['Your recipes', 'Saved workflows, ready for the next time you need them.'],
    runs: ['Your runs', 'Follow work in progress and revisit the results.'],
    connections: ['Connections', 'The tools and services available to this project.'],
    costs: ['Costs', 'See what CookBook has recorded for a complete billing period.'],
    schedules: ['Schedules', 'Choose when a recipe runs again.'],
    activity: ['Activity & API calls', 'Detailed records for troubleshooting and reviewing usage.'],
    recipe: ['Recipe details', 'Review the steps and provide the inputs for a new run.'],
    run: ['Run details', 'Follow each step, review results, and continue saved work.'],
    advanced: ['Recipe author tools', 'Technical controls for registering recipes and project settings.'],
    'app-preview': ['App previews', 'Open a saved tool result in an isolated, read-only window.'],
  };
  const aliases = {top:'home', kitchen:'home', catalog:'recipes', workflow:'recipe', details:'run', 'billing-period':'costs', analytics:'activity', author:'advanced', 'mcp-app-preview':'app-preview'};
  let routeEpoch = 0;
  async function navigate(page, id = '', navigation = 'new') {
    if (!pages.length) return;
    const hash = '#' + page + (id ? '=' + encodeURIComponent(id) : '');
    if (location.hash !== hash) history.pushState(null, '', hash);
    await renderPage(navigation);
  }
  const openDetail = (selection, navigation = 'new') => navigate(selection.run_id ? 'run' : 'recipe', selection.run_id || selection.process_ref, navigation);
  async function renderPage(navigation = 'new') {
    const epoch = ++routeEpoch;
    const [rawPage, ...rawId] = location.hash.slice(1).split('=');
    let page = aliases[rawPage] || rawPage || 'home';
    if (!Object.hasOwn(pageInfo, page) || (page === 'advanced' && !isAuthor) || (page === 'app-preview' && !window.aprDesktopCall)) page = 'home';
    let id;
    try { id = decodeURIComponent(rawId.join('=')); } catch { id = ''; }
    const detail = page === 'recipe' || page === 'run';
    if (detail && !id) { page = page === 'run' ? 'runs' : 'recipes'; }
    const visiblePage = page === 'run' ? 'recipe' : page;
    pages.forEach(node => { node.hidden = node.dataset.page !== visiblePage; });
    const active = page === 'recipe' ? 'recipes' : page === 'run' ? 'runs' : page;
    document.querySelectorAll('[data-nav]').forEach(link => {
      if (link.dataset.nav === active) { link.setAttribute('aria-current', 'page'); const group = link.closest('details'); if (group) group.open = true; }
      else link.removeAttribute('aria-current');
    });
    $('page-title').textContent = pageInfo[page][0];
    $('page-description').textContent = pageInfo[page][1];
    document.title = 'CookBook · ' + pageInfo[page][0];
    $('operation-result').hidden = true; $('operation-result').open = false;
    $('page-title').focus({preventScroll:true});
    window.scrollTo({top:0, behavior:'instant'});
    if ((page !== 'recipe' && page !== 'run') || !id) {
      const load = {home:attention, recipes:discoverRecipes, runs:historyPage, connections:connectionsPage, schedules:schedulesPage, activity:analyticsPage}[page];
      if (load) await load();
      return;
    }
    const isRun = page === 'run';
    $('recipe-improvements').hidden = isRun;
    $('recipe-ingredients').hidden = isRun;
    $('details').hidden = !isRun;
    $('detail-back').href = isRun ? '#runs' : '#recipes';
    $('detail-back').textContent = isRun ? '← All runs' : '← All recipes';
    if (isRun) {
      await openGraph({run_id:id}, navigation);
      if (epoch !== routeEpoch) return;
      await inspect('run');
    } else {
      if ($('process-ref').value !== id) {
        $('run-input').value = '{}'; $('run-config').value = '{}';
        $('request-key').value = crypto.randomUUID();
      }
      $('process-ref').value = id; $('config-process-ref').value = id;
      $('ingredient-fields').replaceChildren();
      const [, schema] = await Promise.all([openGraph({process_ref:id}, navigation), call('discover', {process_ref:id, detail:'schema'}, true)]);
      if (epoch !== routeEpoch) return;
      ingredientForm(schema.input_schema || schema.schema?.input_schema);
      await readiness(id);
      if (epoch === routeEpoch) await improvementReviews(id, epoch);
    }
  }
  async function improvementReviews(processRef, epoch = routeEpoch) {
    const list = $('improvement-list'), report = $('improvement-report');
    list.replaceChildren(element('p', 'Loading improvement reviews…'));
    report.replaceChildren();
    try {
      const result = await call('inspect', {view:'improvements', process_ref:processRef}, true);
      if (epoch !== routeEpoch) return;
      list.replaceChildren();
      for (const cycle of result.cycles || []) {
        const label = cycle.state === 'closed' ? (cycle.outcome === 'no_supported_improvement' ? 'No supported change' : 'Needs more evidence') : cycle.state === 'evaluated' ? ({pass:'Ready for review', fail:'Did not pass', inconclusive:'Needs more evidence'}[cycle.result] || 'Evaluated') : ({adopted:'Adopted', rejected:'Rejected', opened:'Review started', proposed:'Candidate proposed', frozen:'Ready to evaluate'}[cycle.state] || friendlyState(cycle.state));
        const row = actionButton(`${label} · ${shortRef(cycle.candidate_ref || cycle.baseline_ref)}`, async () => {
          report.replaceChildren(element('p', 'Loading this review…'));
          try {
            const value = await call('inspect', {view:'improvement', cycle_id:cycle.cycle_id}, true);
            if (epoch !== routeEpoch) return;
            report.replaceChildren(element('h3', value.proposal?.hypothesis || 'Recipe improvement review'));
            if (value.proposal?.change) report.append(element('p', value.proposal.change));
            const evaluation = value.report;
            if (!evaluation) { report.append(element('p', value.state === 'closed' ? value.reason : 'Evaluation is not complete. Continue this review with your agent.')); return; }
            const verdict = value.state === 'adopted' ? 'Adopted candidate' : value.state === 'rejected' ? 'Rejected candidate' : evaluation.eligible ? 'Eligible for your review' : evaluation.result === 'fail' ? 'Candidate did not pass' : 'More evidence is needed';
            report.append(element('h4', verdict));
            if (evaluation.reason) report.append(element('p', evaluation.reason));
            report.append(element('p', `Complete expected outputs: baseline ${evaluation.baseline_correct ?? 'unknown'}, candidate ${evaluation.candidate_correct ?? 'unknown'}, from ${evaluation.evaluated_cases ?? 0} of ${evaluation.heldout_cases ?? 0} frozen cases.`));
            report.append(element('p', `Executed steps: baseline ${evaluation.baseline_steps_total ?? 'unknown'}; candidate ${evaluation.candidate_steps_total ?? 'unknown'}.`));
            report.append(element('p', evaluation.quality_scope || 'Quality is limited to the frozen labeled cases.'));
            report.append(element('p', 'Provider cost, caller model usage and live latency are not measured by this local evaluation.'));
            report.append(element('p', value.state === 'adopted' ? 'Adopted for future work. Existing runs and schedules keep their saved versions.' : value.state === 'rejected' ? 'Rejected. The selected recipe version has not changed.' : 'Discuss this report with your agent. Adopting a candidate requires your explicit approval.'));
            const identity = element('details');
            identity.append(element('summary', 'Versions, policy and report identity'), element('pre', JSON.stringify({cycle_id:value.cycle_id, baseline_ref:value.baseline_ref, candidate_ref:value.candidate_ref, policy_digest:value.policy_digest, report_digest:value.report_digest, state:value.state, mode:evaluation.mode}, null, 2)));
            report.append(identity);
          } catch (error) { if (epoch === routeEpoch) report.replaceChildren(element('p', `Could not load this review: ${error.message}`)); }
        });
        row.className = 'list-item'; list.append(row);
      }
      if (!list.childElementCount) list.append(element('p', 'No improvement reviews yet. Ask your agent to review this recipe’s recent runs when you want to improve it.'));
    } catch (error) {
      const recovery = window.aprDesktopCall ? 'Open Settings & access and reopen this workspace, then refresh.' : 'Open a fresh inspector session and try again.';
      if (epoch === routeEpoch) list.replaceChildren(element('p', `Could not load reviews: ${error.message.replace(/[.!?]+$/, '')}. ${recovery}`));
    }
  }
  $('improvement-refresh').addEventListener('click', () => work(() => improvementReviews($('process-ref').value)));
  let cursor = null;
  let scheduleCursor = null;
  document.querySelectorAll('[data-author]').forEach(element => { element.hidden = document.body.dataset.mode !== 'author'; });
  const show = (title, value) => { $('result-title').textContent = title + ' · technical details'; $('result').textContent = JSON.stringify(value, null, 2); $('operation-result').hidden = false; };
  const notice = message => { $('notice').textContent = message; };
  const parse = id => JSON.parse($(id).value);
  async function call(action, args, detail = false) {
    if (window.aprDesktopCall) {
      const value = await window.aprDesktopCall(action,args,detail);
      $('connection').textContent = 'Local service connected';
      $('connection').classList.add('connected');
      return value;
    }
    if (!token) throw new Error('Open the session URL printed by apr inspector. Reloading the cleared URL starts without a session.');
    const response = await fetch('/api', { method: 'POST', credentials: 'omit', cache: 'no-store', headers: { 'Content-Type': 'application/json', 'X-APR-Session': token }, body: JSON.stringify({ action, args, detail }) });
    const value = await response.json();
    if (!response.ok || value.error) throw new Error(value.error?.code || 'REQUEST_FAILED');
    $('connection').textContent = 'Local service connected';
    $('connection').classList.add('connected');
    return value.result;
  }
  const work = async operation => { notice(''); try { await operation(); } catch (error) { notice(error.message); $('connection').textContent = 'Attention needed'; $('connection').classList.remove('connected'); } };
  let graphSelection = null;
  let graphData = null;
  let graphTrail = [];
  let graphEpoch = 0;
  let treeCursor = null;
  let treeEpoch = 0;
  let tuningModel = null;
  let tuningEpoch = 0;
  let inspectEpoch = 0;
  let submissionRun = null;
  let tuningReaders = [];
  let tuningPreview = null;
  const element = (tag, text, className) => {
    const node = document.createElement(tag);
    if (text !== undefined) node.textContent = text;
    if (className) node.className = className;
    return node;
  };
  const friendlyState = state => (state || 'definition').replaceAll('_', ' ');
  const shortRef = ref => ref ? ref.slice(0, 18) + '…' : '';
  const isAuthor = document.body.dataset.mode === 'author';
  function actionButton(label, action) {
    const button = element('button', label); button.type = 'button';
    button.addEventListener('click', () => work(action)); return button;
  }
  function closeTuning() {
    tuningEpoch++;
    tuningModel = null; tuningReaders = []; tuningPreview = null;
    $('jev-tuning').hidden = true; $('tuning-publish').disabled = true;
    $('tuning-form').querySelectorAll('button').forEach(button => { button.disabled = button.id === 'tuning-publish'; });
    $('tuning-review-panel').hidden = true; $('tuning-review').textContent = '';
    $('tuning-fields').replaceChildren(); $('tuning-status').textContent = '';
  }
  function invalidateTuning() {
    tuningPreview = null; $('tuning-publish').disabled = true; $('tuning-review-panel').hidden = true;
    $('tuning-status').textContent = 'Changes need a new preview before publishing.';
  }
  async function openGraph(selection, navigation = 'new') {
    const epoch = ++graphEpoch;
    graphData = null; treeEpoch++;
    $('graph-stage').replaceChildren(element('p', 'Loading recipe…', 'empty'));
    $('node-panel').replaceChildren(element('h3', 'Step details'), element('p', 'Loading selected recipe…'));
    $('tree-list').replaceChildren();
    inspectEpoch++; submissionRun = null; $('submission').value = '';
    $('run-id').value = selection.run_id || '';
    closeTuning();
    const value = await call('inspect', { view: 'graph', ...selection, limit: 100 }, true);
    if (epoch !== graphEpoch) return;
    graphSelection = { ...selection }; graphData = value;
    if (navigation === 'new') graphTrail = [];
    if (navigation !== 'refresh') graphTrail.push({ selection: { ...selection }, name: value.process_name || shortRef(value.process_ref) });
    $('graph-title').textContent = value.process_name || 'Recipe';
    $('graph-summary').textContent = value.run_id
      ? `Saved execution · ${friendlyState(value.state)}. Select a step to see its state or open nested work.`
      : 'The saved steps this recipe will follow. Select a step to see what it does.';
    $('graph-identity').textContent = `${value.run_id ? value.run_id + ' · ' : ''}${value.process_ref}${value.truncated || selection.after ? ' · Partial graph page; dependencies outside this page are not drawn.' : ''}`;
    $('graph-refresh').disabled = false; $('graph-next').disabled = !value.truncated || !value.next_cursor;
    $('graph-breadcrumbs').replaceChildren();
    graphTrail.forEach((entry, index) => {
      const button = actionButton(entry.name, async () => {
        graphTrail = graphTrail.slice(0, index);
        await openDetail(entry.selection, 'child');
      });
      button.setAttribute('aria-current', index === graphTrail.length - 1 ? 'location' : 'false');
      $('graph-breadcrumbs').append(button);
    });
    renderGraph(value);
    $('run-family').hidden = !value.run_id;
    if (value.run_id) {
      $('run-id').value = value.run_id;
      await loadTree(false, epoch);
    } else { treeEpoch++; $('tree-list').replaceChildren(); }
  }
  function renderGraph(value) {
    const stage = $('graph-stage'); stage.replaceChildren();
    const svg = document.createElementNS('http://www.w3.org/2000/svg', 'svg');
    svg.classList.add('graph-svg'); svg.setAttribute('aria-hidden', 'true'); stage.append(svg);
    const nodes = value.nodes || []; const byId = new Map(nodes.map(node => [node.id, node]));
    const levels = new Map(); const visiting = new Set();
    function level(node) {
      if (levels.has(node.id)) return levels.get(node.id);
      if (visiting.has(node.id)) return 0;
      visiting.add(node.id);
      const deps = (node.needs || []).filter(id => byId.has(id));
      const result = deps.length ? Math.max(...deps.map(id => level(byId.get(id)))) + 1 : 0;
      visiting.delete(node.id); levels.set(node.id, result); return result;
    }
    const columns = new Map(); const buttons = new Map();
    [...nodes].sort((a, b) => level(a) - level(b)).forEach(node => {
      const columnId = level(node);
      if (!columns.has(columnId)) { const column = element('div', undefined, 'graph-layer'); columns.set(columnId, column); stage.append(column); }
      const button = actionButton('', async () => {
        buttons.forEach(item => item.setAttribute('aria-pressed', 'false'));
        button.setAttribute('aria-pressed', 'true'); showNode(node);
      });
      button.className = 'graph-node'; button.dataset.state = node.state || 'definition'; button.setAttribute('aria-pressed', 'false');
      button.append(element('span', node.editable_jev ? 'Jev decision' : node.call ? 'Nested recipe' : ({connector:'Use a tool', caller:'Ask your agent', decision:'Choose a path'}[node.kind] || node.kind), 'node-kind'), element('strong', node.id), element('span', friendlyState(node.state), 'node-state'));
      if (node.when) button.append(element('small', `When ${node.when.decision} → ${node.when.route}`));
      if (node.foreach) button.append(element('small', 'Repeated for each item'));
      columns.get(columnId).append(button); buttons.set(node.id, button);
    });
    if (!nodes.length) stage.append(element('p', 'No steps in this view.', 'empty'));
    $('node-panel').replaceChildren(element('h3', 'Step details'), element('p', 'Select a step to see its details. Ask your agent to change the recipe.'));
    function drawEdges() {
      if (!stage.isConnected) return;
      svg.replaceChildren(); svg.setAttribute('width', String(stage.scrollWidth)); svg.setAttribute('height', String(stage.scrollHeight));
      const origin = stage.getBoundingClientRect();
      for (const edge of value.edges || []) {
        if (!buttons.has(edge.from) || !buttons.has(edge.to)) continue;
        const from = buttons.get(edge.from).getBoundingClientRect(); const to = buttons.get(edge.to).getBoundingClientRect();
        const x1 = from.right - origin.left; const y1 = from.top + from.height / 2 - origin.top;
        const x2 = to.left - origin.left; const y2 = to.top + to.height / 2 - origin.top;
        const middle = (x1 + x2) / 2;
        const path = document.createElementNS(svg.namespaceURI, 'path'); path.setAttribute('d', `M${x1},${y1} C${middle},${y1} ${middle},${y2} ${x2},${y2}`); path.classList.add('graph-edge'); if (edge.route && byId.get(edge.from)?.selected_route) path.dataset.selected = String(edge.route === byId.get(edge.from).selected_route); svg.append(path);
        const arrow = document.createElementNS(svg.namespaceURI, 'path'); arrow.setAttribute('d', `M${x2 - 5},${y2 - 4} L${x2},${y2} L${x2 - 5},${y2 + 4}`); arrow.classList.add('graph-edge'); svg.append(arrow);
        if (edge.route) { const label = document.createElementNS(svg.namespaceURI, 'text'); label.setAttribute('x', String(middle)); label.setAttribute('y', String((y1 + y2) / 2 - 7)); label.setAttribute('text-anchor', 'middle'); label.classList.add('graph-edge-label'); label.textContent = edge.route; svg.append(label); }
      }
    }
    if (stage._resizeObserver) stage._resizeObserver.disconnect();
    stage._resizeObserver = new ResizeObserver(drawEdges); stage._resizeObserver.observe(stage);
    requestAnimationFrame(drawEdges);
  }
  function showNode(node) {
    const panel = $('node-panel'); panel.replaceChildren(element('h3', node.id));
    const detail = (label, value) => { panel.append(element('div', label, 'detail-label'), element('p', value)); };
    detail('Step', `${node.kind} · ${friendlyState(node.state)}`);
    if (node.needs?.length) detail('Runs after', node.needs.join(', '));
    if (node.when) detail('Selected route', `${node.when.decision} → ${node.when.route}`);
    if (node.decision) {
      detail('Decision', node.decision.source === 'rules' ? 'Exact rules' : 'Typed choice');
      detail('Declared routes', (node.decision.routes || []).join(' · '));
      if (node.selected_route) detail('Recorded selection', node.selected_route);
      if (node.decision.min_confidence !== undefined && node.decision.min_confidence !== null) detail('Minimum confidence', String(node.decision.min_confidence));
      if (node.decision.needs_clarification) detail('When clarification is needed', node.decision.needs_clarification);
    }
    if (node.join) detail('Join', `Collects the selected route from ${node.join.decision}`);
    if (node.call) {
      detail('Pinned child version', node.call.process_ref);
      panel.append(actionButton('Open child recipe definition', async () => openDetail({process_ref:node.call.process_ref}, 'child')));
    }
    for (const instance of node.instances || []) {
      if (instance.child_run_id) {
        panel.append(actionButton(`Open nested run · ${instance.step_instance_id}`, async () => {
          await openDetail({run_id:instance.child_run_id}, 'child');
        }));
      } else if (instance.state === 'waiting_for_agent' && graphData.run_id) {
        panel.append(actionButton('Open this saved review', async () => { await openDetail({run_id:graphData.run_id}); $('details').scrollIntoView({behavior:'smooth'}); }));
      }
    }
    if (node.instances_truncated) panel.append(element('p', 'More item instances exist. Use Nested runs to browse the run family.'));
    if (isAuthor && node.editable_jev) panel.append(actionButton('Tune Jev decisions', async () => openTuning(graphData.process_ref, node.id)));
    if (!node.editable_jev) panel.append(element('p', 'Structure is defined by the process. Ask your agent to change this step.'));
    if (graphData.run_id && node.editable_jev) panel.append(element('p', 'Tuning publishes a new catalog version. This saved run will not change.'));
  }
  async function loadTree(next = false, expectedGraph = graphEpoch) {
    const requestEpoch = ++treeEpoch;
    if (!graphData?.run_id) return;
    const args = { view: 'tree', run_id: graphData.run_id, limit: 25 };
    if (next && treeCursor) args.after = treeCursor;
    const value = await call('inspect', args, true);
    if (expectedGraph !== graphEpoch || requestEpoch !== treeEpoch) return;
    treeCursor = value.next_cursor; $('tree-next').disabled = !value.truncated || !treeCursor;
    const list = $('tree-list'); list.replaceChildren();
    for (const run of value.runs || []) {
      const row = element('div', undefined, 'tree-row');
      row.style.marginLeft = `${Math.min(run.depth || 0, 8) * 10}px`;
      row.append(actionButton(`${run.depth ? '↳ ' : ''}${run.process_name || shortRef(run.process_ref)} · ${friendlyState(run.state)}`, async () => {
        await openDetail({run_id:run.run_id}, 'child');
      }), element('small', `Level ${run.depth || 0} · ${run.run_id}`));
      if (run.actionable_waiting) row.append(element('span', 'Review needed', 'tree-review'));
      list.append(row);
    }
  }
  $('graph-refresh').addEventListener('click', () => work(async () => { if (graphSelection) await openGraph(graphSelection, 'refresh'); }));
  $('graph-next').addEventListener('click', () => work(async () => { if (graphSelection && graphData?.next_cursor) await openGraph({ ...graphSelection, after: graphData.next_cursor }, 'refresh'); }));
  $('tree-next').addEventListener('click', () => work(() => loadTree(true)));
  $('tuning-close').addEventListener('click', closeTuning);

  function formField(group, label, initial, options = {}) {
    const wrapper = element('label', label);
    let input;
    if (options.options) {
      input = element('select');
      options.options.forEach(value => { const option = element('option', value); option.value = value; input.append(option); });
      input.value = initial;
    } else if (options.number) {
      input = element('input'); input.type = 'number'; input.min = '0'; input.max = '1'; input.step = 'any'; input.value = initial ?? '';
    } else {
      input = element('textarea'); input.rows = 3;
      input.value = typeof initial === 'string' ? initial : JSON.stringify(initial, null, 2);
      if (typeof initial !== 'string') wrapper.append(element('small', 'Structured value (JSON)', 'tuning-note'));
    }
    wrapper.append(input); group.append(wrapper);
    input.addEventListener('input', invalidateTuning); input.addEventListener('change', invalidateTuning);
    return () => {
      if (options.options) return input.value;
      if (options.number) {
        if (!input.value.trim()) return null;
        const value = Number(input.value); if (!Number.isFinite(value) || value < 0 || value > 1) throw new Error('Confidence must be between 0 and 1.');
        return value;
      }
      return typeof initial === 'string' ? input.value : JSON.parse(input.value);
    };
  }
  function collectTuningChanges() {
    return tuningReaders.map(reader => reader()).filter(change => change !== null);
  }
  async function openTuning(processRef, selectedStep) {
    const epoch = graphEpoch;
    closeTuning(); const requestEpoch = ++tuningEpoch;
    const model = await call('inspect', { view: 'jev_tuning', process_ref: processRef }, true);
    if (epoch !== graphEpoch || requestEpoch !== tuningEpoch || graphData?.process_ref !== processRef) return;
    tuningModel = model; $('jev-tuning').hidden = false;
    const fields = $('tuning-fields');
    for (const entry of model.questions || []) {
      const group = element('fieldset', undefined, 'tuning-group');
      group.append(element('legend', `${entry.step_id} · ${entry.question_id} · ${entry.question.type}`));
      const question = JSON.parse(JSON.stringify(entry.question));
      const readInstructions = formField(group, 'Question / instructions', question.instructions);
      const criteriaReaders = [];
      if (Array.isArray(question.criteria)) {
        question.criteria.forEach((value, index) => criteriaReaders.push([index, formField(group, `Score level ${index + 1}`, value)]));
      } else if (question.criteria && typeof question.criteria === 'object') {
        Object.entries(question.criteria).forEach(([key, value]) => criteriaReaders.push([key, formField(group, `Meaning of ${key}`, value)]));
      }
      group.append(element('p', 'Question type and choice identifiers stay fixed to preserve downstream bindings.', 'tuning-note'));
      tuningReaders.push(() => {
        const updated = JSON.parse(JSON.stringify(question)); updated.instructions = readInstructions();
        for (const [key, read] of criteriaReaders) updated.criteria[key] = read();
        return JSON.stringify(updated) === JSON.stringify(question) ? null : { kind: 'question', step_id: entry.step_id, question_id: entry.question_id, question: updated };
      });
      fields.append(group);
    }
    for (const entry of model.policies || []) {
      const policy = JSON.parse(JSON.stringify(entry.policy));
      const group = element('fieldset', undefined, 'tuning-group'); group.append(element('legend', `${entry.step_id} · routing rules`));
      const reads = [];
      if (policy.source === 'choice') {
        const threshold = formField(group, 'Minimum confidence (blank: no threshold)', policy.min_confidence, { number: true });
        reads.push(updated => { const value = threshold(); if (value === null) delete updated.min_confidence; else updated.min_confidence = value; });
        for (const [key, label] of [['no_match', 'Route when no choice matches'], ['needs_clarification', 'Route when confidence is insufficient']]) {
          const read = formField(group, label, policy[key], { options: policy.routes }); reads.push(updated => { updated[key] = read(); });
        }
      } else if (policy.source === 'rules') {
        policy.cases.forEach((rule, index) => {
          const equals = formField(group, `Rule ${index + 1}: exact matching value`, rule.equals);
          const route = formField(group, `Rule ${index + 1}: route`, rule.route, { options: policy.routes });
          reads.push(updated => { updated.cases[index].equals = equals(); updated.cases[index].route = route(); });
        });
        const fallback = formField(group, 'Default route', policy.default, { options: policy.routes }); reads.push(updated => { updated.default = fallback(); });
      }
      group.append(element('p', 'Declared routes, source bindings, graph structure and permissions are preserved.', 'tuning-note'));
      tuningReaders.push(() => {
        const updated = JSON.parse(JSON.stringify(policy)); reads.forEach(read => read(updated));
        return JSON.stringify(updated) === JSON.stringify(policy) ? null : { kind: 'policy', step_id: entry.step_id, policy: updated };
      });
      fields.append(group);
    }
    if (!tuningReaders.length) $('tuning-status').textContent = 'This process has no supported literal Jev questions or associated routing rules to tune.';
    else $('tuning-status').textContent = `Tuning ${selectedStep}. Edit the Jev questions or their routing rules, then preview.`;
    $('jev-tuning').scrollIntoView({ behavior: 'smooth', block: 'start' });
  }
  $('tuning-form').addEventListener('submit', event => { event.preventDefault(); work(async () => {
    if (!tuningModel) return;
    const model = tuningModel; const changes = collectTuningChanges();
    if (!changes.length) { $('tuning-status').textContent = 'No changes to preview.'; $('tuning-publish').disabled = true; return; }
    tuningPreview = null; $('tuning-publish').disabled = true;
    const result = await call('manage', { action: 'process.tune_jev', process_ref: model.process_ref, mode: 'preview', changes }, true);
    if (model !== tuningModel || JSON.stringify(changes) !== JSON.stringify(collectTuningChanges())) return;
    tuningPreview = { changes, process_ref: result.process_ref };
    $('tuning-status').textContent = `Validated ${changes.length} change(s). New version: ${result.process_ref}\nExisting runs and schedules remain unchanged. No provider call was made. Review the proposed changes below before publishing.`;
    $('tuning-review').textContent = JSON.stringify(changes, null, 2); $('tuning-review-panel').hidden = false; $('tuning-review-panel').open = true;
    show('Review Jev changes before publishing', { base_process_ref: model.process_ref, candidate_process_ref: result.process_ref, changes });
    $('tuning-publish').disabled = false;
  }); });
  $('tuning-publish').addEventListener('click', () => work(async () => {
    if (!tuningModel || !tuningPreview) return;
    const model = tuningModel; const preview = tuningPreview;
    if (JSON.stringify(preview.changes) !== JSON.stringify(collectTuningChanges())) { invalidateTuning(); return; }
    const controls = [...$('tuning-form').querySelectorAll('input,textarea,select,button')];
    controls.forEach(control => { control.disabled = true; });
    let result;
    try {
      result = await call('manage', { action: 'process.tune_jev', process_ref: model.process_ref, mode: 'publish', changes: preview.changes, expected_process_ref: preview.process_ref }, true);
    } catch (error) {
      if (model === tuningModel) controls.forEach(control => { control.disabled = false; });
      throw error;
    }
    show('Published new Jev process version', result);
    if (model !== tuningModel) return;
    $('process-ref').value = result.process_ref; $('config-process-ref').value = result.process_ref;
    await openDetail({ process_ref: result.process_ref });
    $('tuning-form').querySelectorAll('button').forEach(button => { button.disabled = button.id === 'tuning-publish'; });
    notice('Published a new version. Ask your agent to update any parent recipe or schedule to use it. Existing runs and configured versions remain unchanged.');
    $('page-title').focus({preventScroll:true});
  }));

  async function historyPage(next = false) {
    const args = { view: 'runs', limit: 10 };
    if (next && cursor) args.after = cursor;
    const value = await call('inspect', args, true);
    cursor = value.next_cursor;
    $('history-next').disabled = !value.truncated || !cursor;
    $('history').replaceChildren();
    for (const run of value.runs || []) {
      const button = document.createElement('button'); button.className = 'list-item';
      const name = document.createElement('strong'); name.textContent = run.process_name || run.run_id;
      const state = document.createElement('span'); state.className = 'state'; state.textContent = run.state;
      const summary = document.createElement('small'); summary.textContent = `${run.summary} · ${run.run_id}${run.parent_run_id ? ' · nested run' : ''}`;
      button.append(name, state, summary);
      button.addEventListener('click', () => work(() => openDetail({run_id:run.run_id})));
      $('history').append(button);
    }
    if (!value.runs?.length) { const empty = document.createElement('p'); empty.className = 'empty'; empty.textContent = 'No saved runs yet. Choose a recipe from Recipes to start your first run.'; $('history').append(empty); }

  }
  async function inspect(view) {
    const run_id = $('run-id').value.trim();
    if (!run_id) throw new Error('Select or enter a run ID first.');
    const epoch = ++inspectEpoch;
    if (view === 'run') { submissionRun = null; $('submission').value = ''; }
    const args = { view, run_id };
    if (view === 'events') { args.after = 0; args.limit = 100; }
    const value = await call('inspect', args, true);
    if (epoch !== inspectEpoch || run_id !== $('run-id').value.trim()) return;
    show(view === 'config' ? 'Frozen run configuration' : view === 'run' ? 'Run details' : `Run ${view}`, value);
    if (view === 'run' && value.request) {
      const request = value.request; submissionRun = value.run_id;
      $('submission').value = JSON.stringify({ action: 'submit', protocol_version: value.protocol_version, run_id: value.run_id, request_id: request.request_id, step_instance_id: request.step_instance_id, generation: request.generation, continuation_token: request.continuation_token, submission_key: '', output: {}, provenance: {} }, null, 2);
    }
  }
  async function schedulesPage(next = false) {
    const args = { view: 'schedules', limit: 10 };
    if (next && scheduleCursor) args.after = scheduleCursor;
    const value = await call('inspect', args, true);
    scheduleCursor = value.next_cursor;
    $('schedule-next').disabled = !value.truncated || !scheduleCursor;
    $('schedule-list').replaceChildren();
    for (const job of value.schedules || []) {
      const row = document.createElement('div'); row.className = 'list-item';
      const inspectJob = document.createElement('button'); inspectJob.textContent = job.name || job.schedule_id;
      inspectJob.addEventListener('click', () => work(async () => show('Scheduled job', await call('inspect', { view: 'schedule', schedule_id: job.schedule_id }, true))));
      const state = document.createElement('span'); state.className = 'state'; state.textContent = job.state;
      row.append(inspectJob, state);
      if (document.body.dataset.mode === 'author') {
        for (const action of ['pause', 'resume', 'delete']) {
          const control = document.createElement('button'); control.textContent = action[0].toUpperCase() + action.slice(1);
          control.setAttribute('aria-label', `${control.textContent} ${job.name || job.schedule_id}`);
          control.addEventListener('click', () => work(async () => {
            if (action === 'delete' && !await (window.aprConfirm || window.confirm)('Delete this schedule? Existing runs and history remain.')) return;
            show('Schedule control', await call('manage', { action: `schedule.${action}`, schedule_id: job.schedule_id }, true));
            await schedulesPage();
          }));
          row.append(control);
        }
      }
      $('schedule-list').append(row);
    }
    if (!value.schedules?.length) {
      const empty = document.createElement('p'); empty.className = 'empty'; empty.textContent = 'No scheduled jobs in this assignment.'; $('schedule-list').append(empty);
    }
  }
  $('schedule-refresh').addEventListener('click', () => work(() => schedulesPage()));
  $('schedule-next').addEventListener('click', () => work(() => schedulesPage(true)));
  $('schedule-kind').addEventListener('change', () => {
    const kind = $('schedule-kind').value;
    $('schedule-value-label').firstChild.textContent = kind === 'interval' ? 'Interval in seconds' : kind === 'once' ? 'Exact time with timezone' : 'Cron expression (UTC)';
    $('schedule-value').value = kind === 'interval' ? '3600' : kind === 'cron' ? '0 15 * * 1-5' : '';
  });
  $('schedule-create').addEventListener('submit', event => { event.preventDefault(); work(async () => {
    const kind = $('schedule-kind').value;
    const raw = $('schedule-value').value.trim();
    let trigger;
    if (kind === 'interval') {
      const seconds = Number(raw);
      if (!Number.isSafeInteger(seconds) || seconds < 1) throw new Error('Enter a positive whole number of seconds.');
      trigger = { kind, every_seconds: seconds };
    } else if (kind === 'once') {
      if (!/^\d{4}-\d\d-\d\dT\d\d:\d\d:\d\d(?:\.\d+)?(?:Z|[+-]\d\d:\d\d)$/.test(raw) || !Number.isFinite(Date.parse(raw))) throw new Error('Enter an exact time with Z or a timezone offset.');
      trigger = { kind, at_ms: Date.parse(raw) };
    } else trigger = { kind, expression: raw };
    const process_ref = $('process-ref').value.trim();
    if (!process_ref) throw new Error('Select a registered process in the catalog first.');
    show('Scheduled job created', await call('manage', { action: 'schedule.create', name: $('schedule-name').value,
      request_key: $('schedule-key').value, process_ref, input: parse('run-input'), config_overrides: parse('run-config'), trigger }, true));
    await schedulesPage();
  }); });
  $('history-refresh').addEventListener('click', () => work(() => historyPage()));
  $('history-next').addEventListener('click', () => work(() => historyPage(true)));
  async function discoverRecipes() {
    const value = await call('discover', {query:$('query').value, limit:20});
    $('catalog-results').replaceChildren();
    for (const process of value.processes || []) {
      const button = element('button', undefined, 'list-item');
      button.append(element('strong', process.name || process.process_name || 'Saved recipe'), element('span', 'View recipe →', 'state'));
      if (process.description) button.append(element('small', process.description));
      if (process.selected_for_future_work) button.append(element('small', 'Selected for future work'));
      button.addEventListener('click', () => work(() => openDetail({process_ref:process.process_ref})));
      $('catalog-results').append(button);
    }
    if (!value.processes?.length) $('catalog-results').append(element('p', $('query').value.trim() ? 'No matching recipes. Try another search.' : 'No recipes here yet. Ask your connected agent to create and register your first recipe.', 'empty'));
  }
  $('discover-form').addEventListener('submit', event => { event.preventDefault(); work(discoverRecipes); });
  $('run-form').addEventListener('submit', event => { event.preventDefault(); work(async () => {
    const value = await call('run', { process_ref: $('process-ref').value.trim(), input: parse('run-input'), config_overrides: parse('run-config'), request_key: $('request-key').value, mode: event.submitter?.value || 'preflight' }, true);
    if (value.run_id) { await openDetail({run_id:value.run_id}); notice('Run saved. Follow its progress below.'); } else { show('Input check', value); $('operation-result').open = value.valid !== true; notice(value.valid === true ? 'Inputs look good. You can start this recipe. No work was run.' : 'Review the input check below before starting.'); } await historyPage();
  }); });
  $('inspect-form').addEventListener('submit', event => { event.preventDefault(); work(async () => { const run_id = $('run-id').value.trim(); await openDetail({run_id}); }); });
  document.querySelectorAll('[data-view]').forEach(button => button.addEventListener('click', () => work(async () => { await inspect(button.dataset.view); $('operation-result').open = true; })));
  document.querySelectorAll('[data-control]').forEach(button => button.addEventListener('click', () => work(async () => {
    const run_id = $('run-id').value.trim(); if (!run_id) throw new Error('Select or enter a run ID first.');
    if (button.dataset.control === 'cancel' && !await (window.aprConfirm || window.confirm)('Stop future work in this run? Completed results remain saved. An external action already sent may still finish; cancellation is not rollback.')) return;
    show('Run control', await call('resume', { action: button.dataset.control, run_id }, true)); await historyPage();
  })));
  $('submit-form').addEventListener('submit', event => { event.preventDefault(); work(async () => { const submission = parse('submission'); if (!submissionRun || submission.run_id !== submissionRun || submission.run_id !== $('run-id').value.trim()) throw new Error('Inspect the run with this pending review before submitting its result.'); const receipt = await call('resume', submission, true); if (submissionRun === submission.run_id && $('run-id').value.trim() === submission.run_id) { show('Submission receipt', receipt); submissionRun = null; $('submission').value = ''; } await historyPage(); if (graphSelection?.run_id === submission.run_id) await openGraph(graphSelection, 'refresh'); }); });
  $('artifact-form').addEventListener('submit', event => { event.preventDefault(); work(async () => show('Artifact preview', await call('inspect', { view: 'artifact', reference: parse('artifact-reference') }, true))); });
  $('manage-form').addEventListener('submit', event => { event.preventDefault(); work(async () => {
    const args = { action: $('manage-action').value, path: $('manage-path').value }; if (args.action === 'artifact.import') args.media_type = $('media-type').value;
    const value = await call('manage', args, true); show('Author operation', value); if (value.process_ref) { $('process-ref').value = value.process_ref; $('config-process-ref').value = value.process_ref; }
  }); });
  $('config-form').addEventListener('submit', event => { event.preventDefault(); work(async () => {
    const args = { action: event.submitter?.value === 'set' ? 'config.set' : 'config.read', process_ref: $('config-process-ref').value.trim() };
    if (args.action === 'config.set') args.values = parse('config-values'); show('Assignment settings', await call('manage', args, true));
  }); });
  let analyticsCursor = null;
  let analyticsEpoch = 0;
  let analyticsSelection = null;
  function amount(nanos, currency) {
    if (nanos === null || nanos === undefined) return 'Unknown';
    const n = BigInt(nanos);
    const whole = n / 1000000000n;
    const fraction = (n % 1000000000n).toString().padStart(9, '0').replace(/0+$/, '').padEnd(2, '0');
    return `${currency} ${whole}.${fraction}`;
  }
  async function analyticsPage(next = false) {
    const epoch = ++analyticsEpoch;
    const run = $('analytics-scope').value === 'run' ? $('run-id').value.trim() : null;
    if ($('analytics-scope').value === 'run' && !run) throw new Error('Select a saved run first.');
    if (!next || run !== analyticsSelection) analyticsCursor = null;
    analyticsSelection = run;
    $('analytics-next').disabled = true;
    const value = await call('inspect', { view: 'analytics', limit: 8, ...(run ? { run_id: run } : {}), ...(analyticsCursor ? { after: analyticsCursor } : {}) }, true);
    if (epoch !== analyticsEpoch) return;
    analyticsCursor = value.next_cursor;
    $('analytics-next').disabled = !value.truncated || !analyticsCursor;
    const summary = value.summary;
    const roots = Object.values(summary.root_run_states).reduce((a, b) => a + b, 0);
    const mean = summary.mean_finished_attempt_ms == null ? 'Unknown' : `${Math.round(summary.mean_finished_attempt_ms)} ms`;
    $('analytics-summary').replaceChildren(
      element('p', `${roots} root runs · ${summary.child_runs} child runs · ${summary.attempts} recorded attempts · ${summary.retries} retries`),
      element('p', `${summary.pending_handoffs} pending handoffs · ${summary.external_attempts} external attempts · Mean finished-attempt duration: ${mean}`),
      element('p', Object.entries(summary.root_run_states).map(([state, count]) => `${friendlyState(state)}: ${count}`).join(' · ') || 'No saved runs in this scope.')
    );
    $('analytics-totals').replaceChildren(element('p', `${value.cost_page.unknown_or_partial_calls} calls on this page have missing or partial cost data. ${value.cost_page.not_sent_calls} were not sent.`));
    for (const total of value.cost_page.currencies) {
      $('analytics-totals').append(element('p', `Page estimate: ${amount(total.estimated_nanos, total.currency)} · Separately, provider reported: ${amount(total.reported_nanos, total.currency)}`));
    }
    $('analytics-calls').replaceChildren();
    for (const call of value.calls) {
      const row = element('div', undefined, 'list-item');
      const usage = call.usage?.observation;
      row.append(element('strong', `${call.connection} · ${friendlyState(call.status)}`));
      row.append(element('p', `${call.run_id} / ${call.step_instance_id}`));
      row.append(element('p', usage ? `Estimate: ${amount(usage.estimated_nanos, usage.currency)} (${usage.estimate_status}, configured meters only) · Provider reported: ${amount(usage.reported_nanos, usage.currency)}` : 'Cost unknown: no usage observation is available.'));
      if (usage) {
        row.append(element('p', `Source: ${friendlyState(call.usage.provenance)} · Pricing revision: ${usage.pricing_revision}`));
        const details = element('details'); details.append(element('summary', 'Usage breakdown'));
        for (const meter of usage.meters) details.append(element('p', `${meter.name}: ${meter.quantity ?? 'Unknown'} ${meter.unit} · ${meter.basis || 'response'} basis${meter.issue ? ' · ' + friendlyState(meter.issue) : ''}`));
        row.append(details);
      }
      $('analytics-calls').append(row);
    }
    if (!value.calls.length) $('analytics-calls').append(element('p', 'No external calls on this page. No cost savings are inferred.'));
  }
  async function connectionsPage() {
    const value = await call('inspect', { view: 'bindings' }, true);
    $('connections-list').replaceChildren();
    for (const binding of value.bindings || []) {
      const row=element('div');
      row.append(element('p', `${binding.name} · ${binding.kind} · revision ${binding.revision} · ${binding.enabled ? 'Enabled' : 'Revoked'}`));
      if(binding.kind==='connection') row.append(actionButton('Inspect payload & cost contract',async()=>show('Connection contract',await call('inspect',{view:'connection_contract',name:binding.name},true))));
      $('connections-list').append(row);
    }
    if (!value.bindings?.length) $('connections-list').append(element('p', 'No tools are approved for this project yet. Ask your agent for the connection your recipe needs, then review its request in Settings & access.'));
  }
  $('analytics-refresh').addEventListener('click', () => work(() => analyticsPage()));
  $('analytics-next').addEventListener('click', () => work(() => analyticsPage(true)));
  $('analytics-scope').addEventListener('change', () => work(() => analyticsPage()));
  $('connections-refresh').addEventListener('click', () => work(() => connectionsPage()));

  async function attention(){
    const [waiting,approvals]=await Promise.all([call('inspect',{view:'pending',limit:8},true),call('inspect',{view:'approvals'},true)]);
    const list=$('attention-list');list.replaceChildren();
    for(const run of waiting.runs||[]){
      const row=element('article');row.append(element('strong','Waiting for your agent'),element('p',run.summary||run.run_id));
      row.append(actionButton('Review waiting step',async()=>{ await openDetail({run_id:run.run_id});}));list.append(row);
    }
    for(const request of approvals.requests||[]){if(request.state!=='pending')continue;
      const row=element('article');row.append(element('strong',`Access requested: ${request.name}`),element('p',request.rationale));
      const detail=element('details');detail.append(element('summary','Requested action and pricing assumptions'),element('pre',JSON.stringify(request.connection,null,2)));row.append(detail);
      if(window.aprOpenSetup)row.append(actionButton('Review access request',window.aprOpenSetup));
      else row.append(element('p','Open CookBook desktop Setup & approvals, or use the trusted owner CLI. This inspector cannot grant access.'));
      list.append(row);
    }
    if(!list.childElementCount)list.append(element('p','No waiting agent steps or access requests here. Check Runs for failures or other work that may need attention.'));
    if(waiting.truncated||approvals.truncated)list.append(element('p','This is a bounded attention list. Resolve shown items and refresh for more.'));
  }
  async function readiness(ref){
    const value=await call('inspect',{view:'readiness',process_ref:ref},true);
    const state=value.can_finish_unattended?'No known agent or approval blockers.':'This recipe needs attention before unattended execution.';
    notice(`${state} ${value.agent_steps} agent-dependent steps; ${value.unpriced_connections} connections without a cost contract. Static check only: it does not verify live accounts or server availability.`);
    return value;
  }
  function ingredientForm(schema){
    const box=$('ingredient-fields');box.replaceChildren();
    if(!schema||schema.type!=='object'||Object.keys(schema.properties||{}).length>24){box.append(element('p','Use the advanced JSON ingredients for this recipe.'));return;}
    const initial=parse('run-input');
    const properties=Object.entries(schema.properties||{});
    if(!properties.length){box.append(element('p','No simple ingredient fields declared. Your agent supplies structured ingredients; JSON remains available below.'));return;}
    box.append(element('h3','Ingredients'));
    for(const [name,spec] of properties){
      if(!['string','integer','number','boolean'].includes(spec.type)){box.append(element('p',`${name}: edit this structured ingredient in JSON below.`));continue;}
      const label=element('label',spec.title||name);
      const input=spec.enum?element('select'):element('input');
      if(spec.enum){input.append(element('option',''));input.firstChild.value='';for(const value of spec.enum){const option=element('option',String(value));option.value=JSON.stringify(value);input.append(option);}}
      else input.type=spec.type==='boolean'?'checkbox':spec.type==='string'?'text':'number';
      if(spec.type==='integer')input.step='1';if(spec.type==='number')input.step='any';
      if(spec.type==='boolean')input.checked=initial[name]===true;
      else input.value=spec.enum?(name in initial?JSON.stringify(initial[name]):''):(initial[name]??'');
      input.addEventListener('input',()=>{try{const value=parse('run-input');if(spec.enum){if(input.value==='')delete value[name];else value[name]=JSON.parse(input.value);}else if(spec.type==='boolean')value[name]=input.checked;else if(input.value==='')delete value[name];else value[name]=spec.type==='string'?input.value:Number(input.value);$('run-input').value=JSON.stringify(value,null,2);}catch(error){notice('Fix the ingredient JSON before editing these fields.');}});
      if(schema.required?.includes(name))label.append(document.createTextNode(' (required)'));
      label.append(input);box.append(label);
    }
    box.append(element('p','These fields update the JSON below. Recipe validation checks required values and constraints before a run.'));
  }
  $('request-key').value=crypto.randomUUID();
  $('new-batch').addEventListener('click',()=>{$('request-key').value=crypto.randomUUID();notice('New run prepared. Starting it can create new external effects and charges.');});
  $('readiness-check').addEventListener('click',()=>work(async()=>show('Recipe readiness',await readiness($('process-ref').value.trim()))));
  $('attention-refresh').addEventListener('click',()=>work(attention));

  if (!pages.length) work(async () => { await attention(); await historyPage(); await schedulesPage(); await analyticsPage(); await connectionsPage(); await discoverRecipes(); });

  const periodStart = new Date(); periodStart.setDate(1); periodStart.setHours(0,0,0,0);
  const periodEnd = new Date(periodStart); periodEnd.setMonth(periodEnd.getMonth()+1);
  function localInput(date) { return new Date(date.getTime()-date.getTimezoneOffset()*60000).toISOString().slice(0,16); }
  $('period-start').value = localInput(periodStart); $('period-end').value = localInput(periodEnd);
  let periodEpoch=0;
  $('billing-period-form').addEventListener('submit', event => { event.preventDefault(); work(async () => {
    const epoch=++periodEpoch;
    try {
      const starts_at_ms=new Date($('period-start').value).getTime(),ends_at_ms=new Date($('period-end').value).getTime();
      if (!Number.isSafeInteger(starts_at_ms)||!Number.isSafeInteger(ends_at_ms)||ends_at_ms<=starts_at_ms) throw new Error('Choose a valid billing period.');
      $('period-status').textContent='Reading complete period…';
      const result=await call('inspect',{view:'billing',starts_at_ms,ends_at_ms},true);
      const policy=await call('inspect',{view:'budget'},true);
      if(epoch!==periodEpoch)return;
      $('period-totals').replaceChildren();
      $('period-status').textContent=`${result.calls} recorded calls · whole period · this assignment. Invoice matches are owner-imported, not independently verified.`;
      for(const group of result.groups||[]) {
        const row=element('article',undefined,'row');
        row.append(element('h3',`${group.connection} · ${group.currency||'Currency unknown'}`));
        row.append(element('p',`${group.calls} calls · ${group.unknown_or_partial} unknown/partial estimates · ${group.not_sent} not sent`));
        row.append(element('p',`Estimated: ${amount(group.estimated_nanos,group.currency||'')} · Provider-reported: ${amount(group.provider_reported_nanos,group.currency||'')} · Invoice-matched: ${amount(group.invoice_matched_nanos,group.currency||'')}`));
        row.append(element('p',`Reserved maximum exposure: ${amount(group.reserved_nanos,group.currency||'')}`));
        $('period-totals').append(row);
      }
      if(!(result.groups||[]).length)$('period-totals').append(element('p','No recorded calls in this period. Costs outside APR are not included.','empty'));
      $('period-budget').textContent=JSON.stringify(policy,null,2);
    } catch (error) {
      if(epoch!==periodEpoch)return;
      $('period-status').textContent='Could not read this billing period.';
      $('period-totals').replaceChildren();
      $('period-budget').textContent='';
      throw error;
    }
  }); });
  if (pages.length) {
    window.addEventListener('hashchange', () => work(renderPage));
    document.querySelector('.skip-link').addEventListener('click', event => { event.preventDefault(); $('page-title').focus(); });
    document.querySelectorAll('[data-open-setup]').forEach(button => button.addEventListener('click', () => work(async () => {
      if (window.aprOpenSetup) await window.aprOpenSetup();
      else notice('Open the CookBook desktop app and choose Settings & access to connect a project or approve tools.');
    })));
    work(renderPage);
  }
})();
