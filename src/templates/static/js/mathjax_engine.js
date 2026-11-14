/**
 * Host-side MathJax Engine.
 * Manages opt-in state (sessionStorage), lazily creates the sandboxed worker iframe,
 * sends batches of extracted expressions, sanitizes returned SVG via DOMParser + allowlist,
 * applies validated SVG styles via CSSOM under strict CSP, and supports lossless restoration.
 */
window.MathJaxEngine = (function () {
    'use strict';

    const STORAGE_KEY = 'ankicollab_mathjax_enabled';
    const WORKER_SRC = '/static/mathjax_sandbox.html';
    const WORKER_READY_TIMEOUT_MS = 15000;
    const BATCH_TIMEOUT_MS = 10000;
    const MAX_CACHE_ENTRIES = 2000;
    const BR_SENTINEL = '\uE000MJXBR\uE001';
    const CONTAINER_SELECTOR = '.note-content-display, .mathjax-editor-preview-body, .comparison-content, .added-content-preview, .removed-content-preview, .denied-content-preview, .snapshot-field';

    const SVG_NS = 'http://www.w3.org/2000/svg';
    const ALLOWED_SVG_TAGS = new Map([
        ['svg', 'svg'],
        ['g', 'g'],
        ['path', 'path'],
        ['rect', 'rect'],
        ['line', 'line'],
        ['circle', 'circle'],
        ['ellipse', 'ellipse'],
        ['polygon', 'polygon'],
        ['polyline', 'polyline'],
        ['text', 'text'],
        ['tspan', 'tspan'],
        ['title', 'title']
    ]);

    const ALLOWED_SVG_ATTRS = new Set([
        'viewBox', 'width', 'height', 'transform', 'd',
        'x', 'y', 'x1', 'y1', 'x2', 'y2', 'cx', 'cy', 'r', 'rx', 'ry', 'points',
        'fill', 'fill-rule', 'stroke', 'stroke-width', 'stroke-linecap',
        'stroke-linejoin', 'stroke-dasharray', 'stroke-dashoffset', 'stroke-miterlimit',
        'opacity', 'fill-opacity', 'stroke-opacity',
        'font-family', 'font-size', 'font-weight', 'font-style',
        'text-anchor', 'dominant-baseline', 'role', 'aria-hidden', 'aria-label',
        'class', 'xmlns', 'data-mml-node'
    ]);

    const ALLOWED_STYLE_PROPS = new Set([
        'vertical-align', 'width', 'height', 'min-width', 'max-width',
        'min-height', 'max-height', 'color', 'fill', 'stroke', 'opacity',
        'margin', 'margin-top', 'margin-bottom', 'margin-left', 'margin-right', 'display'
    ]);

    const SKIP_TAGS = new Set(['SCRIPT', 'STYLE', 'TEXTAREA', 'CODE', 'PRE', 'NOSCRIPT', 'SVG']);

    let workerIframe = null;
    let workerReady = false;
    let workerReadyPromise = null;
    let settleWorkerStartupFn = null;
    let workerStartupTimer = null;
    let messageListenerAttached = false;
    let pinnedSandboxOrigin = null;
    let requestCounter = 0;
    const pendingRequests = new Map();
    const svgCache = new Map();
    const rawFragmentStore = new WeakMap();

    // Sweep legacy persistent localStorage key if previously set.
    try {
        if (window.localStorage) {
            window.localStorage.removeItem(STORAGE_KEY);
        }
    } catch (_) {
        // Ignore storage errors (e.g. private browsing restrictions)
    }

    function isEnabled() {
        try {
            return window.sessionStorage.getItem(STORAGE_KEY) === 'true';
        } catch (_) {
            return false;
        }
    }

    function updateToggleButtons(enabled) {
        const buttons = document.querySelectorAll('.mathjax-toggle-btn');
        for (let i = 0; i < buttons.length; i++) {
            const btn = buttons[i];
            btn.classList.toggle('active', enabled);
            btn.setAttribute('aria-pressed', String(enabled));
        }
    }

    function setEnabled(enabled) {
        const nextState = Boolean(enabled);
        try {
            if (nextState) {
                window.sessionStorage.setItem(STORAGE_KEY, 'true');
            } else {
                window.sessionStorage.removeItem(STORAGE_KEY);
            }
        } catch (_) {
            // Ignore storage errors (e.g. private browsing restrictions)
        }

        updateToggleButtons(nextState);

        if (nextState) {
            renderAll(document);
        } else {
            restoreAll(document);
        }

        document.dispatchEvent(new CustomEvent('mathjax:toggled', {
            detail: { enabled: nextState }
        }));
    }

    function toggle() {
        setEnabled(!isEnabled());
    }

    /**
     * Checks whether plain text or HTML contains at least one closed \(...\) or \[...\] pair.
     * @param {string} text
     * @returns {boolean}
     */
    function containsMathDelimiters(text) {
        if (typeof text !== 'string' || (!text.includes('\\(') && !text.includes('\\['))) {
            return false;
        }
        return findMathMatches(text).length > 0;
    }

    function isUnescapedBackslash(str, idx) {
        if (str[idx] !== '\\') return false;
        let count = 0;
        for (let k = idx - 1; k >= 0 && str[k] === '\\'; k--) {
            count += 1;
        }
        return (count % 2) === 0;
    }

    /**
     * Finds non-overlapping closed \(...\) and \[...\] ranges in a combined text string.
     * If another opener is encountered before the matching closer, treats the earlier opener
     * as unclosed and restarts from the inner opener.
     * @param {string} text
     * @returns {Array<{ start: number, end: number, display: boolean }>}
     */
    function findMathMatches(text) {
        const matches = [];
        let i = 0;
        const len = text.length;

        while (i + 1 < len) {
            let closer = null;
            let display = false;

            if (isUnescapedBackslash(text, i) && text[i + 1] === '(') {
                closer = '\\)';
                display = false;
            } else if (isUnescapedBackslash(text, i) && text[i + 1] === '[') {
                closer = '\\]';
                display = true;
            } else {
                i += 1;
                continue;
            }

            let j = i + 2;
            let closed = false;
            let restarted = false;

            while (j + 1 < len) {
                if (isUnescapedBackslash(text, j) && text[j + 1] === closer[1]) {
                    matches.push({ start: i, end: j + 2, display: display });
                    i = j + 2;
                    closed = true;
                    break;
                }
                if (isUnescapedBackslash(text, j) && (text[j + 1] === '(' || text[j + 1] === '[')) {
                    i = j;
                    restarted = true;
                    break;
                }
                j += 1;
            }

            if (!closed && !restarted) {
                break;
            }
        }

        return matches;
    }

    /**
     * Validates incoming postMessage events from our sandboxed iframe using the Origin API
     * when available, falling back to exact source identity + opaque ('null') origin check.
     * @param {MessageEvent} event
     * @returns {boolean}
     */
    function isValidWorkerEvent(event) {
        if (!workerIframe || !workerIframe.contentWindow || event.source !== workerIframe.contentWindow) {
            return false;
        }
        if (typeof Origin !== 'undefined' && typeof Origin.from === 'function') {
            try {
                const senderOrigin = Origin.from(event);
                if (!senderOrigin.opaque) {
                    return false;
                }
                if (!pinnedSandboxOrigin) {
                    pinnedSandboxOrigin = senderOrigin;
                } else if (
                    typeof pinnedSandboxOrigin.isSameOrigin === 'function' &&
                    !pinnedSandboxOrigin.isSameOrigin(senderOrigin)
                ) {
                    return false;
                }
                return true;
            } catch (_) {
                return false;
            }
        }
        return event.origin === 'null';
    }

    function settleWorkerStartup(ok) {
        if (workerStartupTimer) {
            clearTimeout(workerStartupTimer);
            workerStartupTimer = null;
        }
        if (typeof settleWorkerStartupFn === 'function') {
            const fn = settleWorkerStartupFn;
            settleWorkerStartupFn = null;
            fn(ok);
        }
        workerReadyPromise = null;
    }

    function teardownWorker(resultPayload) {
        settleWorkerStartup(false);
        if (pendingRequests.size) {
            pendingRequests.forEach(function (entry) {
                clearTimeout(entry.timer);
                entry.resolve(resultPayload);
            });
            pendingRequests.clear();
        }
        if (workerIframe && workerIframe.parentNode) {
            workerIframe.parentNode.removeChild(workerIframe);
        }
        workerIframe = null;
        workerReady = false;
        pinnedSandboxOrigin = null;
    }

    function attachWorkerMessageListener() {
        if (messageListenerAttached) return;
        messageListenerAttached = true;

        window.addEventListener('message', function (event) {
            if (!isValidWorkerEvent(event)) return;
            const data = event.data;
            if (!data || typeof data !== 'object') return;

            if (data.type === 'mathjax-worker-ready') {
                workerReady = true;
                settleWorkerStartup(true);
                return;
            }

            if (data.type === 'mathjax-render-result' && typeof data.requestId === 'string') {
                const entry = pendingRequests.get(data.requestId);
                if (!entry) return;
                pendingRequests.delete(data.requestId);
                clearTimeout(entry.timer);
                entry.resolve(data.results || {});
            }
        });
    }

    function ensureWorker() {
        attachWorkerMessageListener();

        if (!document.body) {
            return Promise.resolve(false);
        }

        if (workerIframe && workerIframe.isConnected && workerReady) {
            return Promise.resolve(true);
        }

        if (!workerIframe || !workerIframe.isConnected) {
            workerReady = false;
            workerReadyPromise = null;
            pinnedSandboxOrigin = null;
            const iframe = document.createElement('iframe');
            iframe.setAttribute('sandbox', 'allow-scripts');
            iframe.setAttribute('src', WORKER_SRC);
            iframe.setAttribute('aria-hidden', 'true');
            iframe.setAttribute('tabindex', '-1');
            iframe.className = 'mathjax-sandbox-worker';
            iframe.addEventListener('error', function () {
                teardownWorker(null);
            });
            document.body.appendChild(iframe);
            workerIframe = iframe;
        }

        if (!workerReadyPromise) {
            workerReadyPromise = new Promise(function (resolve) {
                let settled = false;
                settleWorkerStartupFn = function (ok) {
                    if (!settled) {
                        settled = true;
                        resolve(Boolean(ok));
                    }
                };
                workerStartupTimer = setTimeout(function () {
                    teardownWorker(null);
                }, WORKER_READY_TIMEOUT_MS);
            });
        }

        return workerReadyPromise;
    }

    function parseSafeSvgStyles(styleValue) {
        if (typeof styleValue !== 'string' || !styleValue) return [];
        const lower = styleValue.toLowerCase();
        if (lower.includes('url(') || lower.includes('expression(') || lower.includes('javascript:')) {
            return [];
        }
        const safeDeclarations = [];
        const parts = styleValue.split(';');
        for (let i = 0; i < parts.length; i++) {
            const decl = parts[i].trim();
            if (!decl) continue;
            const colonIdx = decl.indexOf(':');
            if (colonIdx <= 0) continue;
            const prop = decl.slice(0, colonIdx).trim().toLowerCase();
            const val = decl.slice(colonIdx + 1).trim();
            if (ALLOWED_STYLE_PROPS.has(prop) && /^[#(),.%\s\w+-]+$/.test(val)) {
                safeDeclarations.push({ prop: prop, val: val });
            }
        }
        return safeDeclarations;
    }

    function sanitizeSvgNode(sourceNode, path, styleBindings) {
        if (!sourceNode || sourceNode.nodeType !== Node.ELEMENT_NODE) {
            return null;
        }
        const canonicalTag = ALLOWED_SVG_TAGS.get(String(sourceNode.localName || sourceNode.tagName).toLowerCase());
        if (!canonicalTag) {
            return null;
        }

        const cleanEl = document.createElementNS(SVG_NS, canonicalTag);
        const attrs = sourceNode.attributes;
        let decls = [];
        for (let i = 0; i < attrs.length; i++) {
            const attr = attrs[i];
            const name = attr.name;
            if (name === 'style' || name === 'data-mjx-style') {
                const parsed = parseSafeSvgStyles(attr.value);
                if (parsed.length) {
                    decls = decls.concat(parsed);
                }
            } else if (ALLOWED_SVG_ATTRS.has(name)) {
                const val = attr.value;
                if (/^(?:javascript:|data:text\/html|vbscript:)/i.test(val.trim())) {
                    continue;
                }
                cleanEl.setAttribute(name, val);
            }
        }
        if (decls.length) {
            styleBindings.push({ path: path, decls: decls });
        }

        for (let child = sourceNode.firstChild; child; child = child.nextSibling) {
            if (child.nodeType === Node.TEXT_NODE && (canonicalTag === 'text' || canonicalTag === 'tspan' || canonicalTag === 'title')) {
                cleanEl.appendChild(document.createTextNode(child.nodeValue || ''));
            } else if (child.nodeType === Node.ELEMENT_NODE) {
                const childIdx = cleanEl.children.length;
                const sanitizedChild = sanitizeSvgNode(child, path.concat(childIdx), styleBindings);
                if (sanitizedChild) {
                    cleanEl.appendChild(sanitizedChild);
                }
            }
        }

        return cleanEl;
    }

    /**
     * Parses untrusted SVG markup via DOMParser (image/svg+xml) and reconstructs a sanitized
     * SVGElement exclusively with allowlisted SVG elements and attributes, returning
     * CSSOM style bindings separately to satisfy strict CSP without inline style attributes.
     * @param {string} svgString
     * @returns {{ svgEl: SVGElement, styleBindings: Array }|null}
     */
    function sanitizeSvgMarkup(svgString) {
        if (typeof svgString !== 'string' || !svgString) {
            return null;
        }
        try {
            const parser = new DOMParser();
            const doc = parser.parseFromString(svgString, 'image/svg+xml');
            if (!doc || doc.querySelector('parsererror')) {
                return null;
            }
            const root = doc.documentElement;
            if (!root || String(root.localName).toLowerCase() !== 'svg') {
                return null;
            }
            const styleBindings = [];
            const cleanRoot = sanitizeSvgNode(root, [], styleBindings);
            if (!cleanRoot) {
                return null;
            }
            cleanRoot.setAttribute('data-mathjax-svg', 'true');
            return { svgEl: cleanRoot, styleBindings: styleBindings };
        } catch (_) {
            return null;
        }
    }

    function sendRenderBatch(items) {
        if (!items.length) {
            return Promise.resolve({});
        }
        return ensureWorker().then(function (ready) {
            if (!ready || !workerIframe || !workerIframe.contentWindow) {
                return null;
            }
            return new Promise(function (resolve) {
                const requestId = 'mjx_' + (++requestCounter);
                const timer = setTimeout(function () {
                    if (!pendingRequests.has(requestId)) return;
                    teardownWorker(null);
                }, BATCH_TIMEOUT_MS);

                pendingRequests.set(requestId, {
                    resolve: resolve,
                    timer: timer
                });

                workerIframe.contentWindow.postMessage(
                    {
                        type: 'mathjax-render-batch',
                        requestId: requestId,
                        items: items
                    },
                    '*'
                );
            });
        });
    }

    function shouldSkipElement(el) {
        if (!el || el.nodeType !== Node.ELEMENT_NODE) return true;
        if (SKIP_TAGS.has(el.tagName)) return true;
        if (el.classList.contains('mathjax-rendered')) return true;
        if (el.classList.contains('trumbowyg-box')) return true;
        if (el.getAttribute('contenteditable') === 'true') return true;
        if (window.SharedUI && typeof el.closest === 'function') {
            const noteContext = el.closest('.note-context');
            if (noteContext && noteContext.getAttribute('data-shared-ui-ready') !== 'true') {
                return true;
            }
        }
        return false;
    }

    function getDiffElementRole(el) {
        if (!el || el.nodeType !== Node.ELEMENT_NODE) return null;
        const tag = el.tagName;
        if (tag === 'DEL' && el.hasAttribute('data-diff')) return 'del';
        if (tag === 'INS' && el.hasAttribute('data-diff')) return 'ins';
        if (tag === 'SPAN') {
            if (el.classList.contains('diff-highlight-del')) return 'del';
            if (el.classList.contains('diff-highlight-ins')) return 'ins';
            if (el.hasAttribute('data-diff-char')) return 'char';
        }
        return null;
    }

    function isFlattenableDiffElement(el) {
        if (!getDiffElementRole(el) || shouldSkipElement(el)) return false;
        const stack = Array.from(el.childNodes);
        while (stack.length > 0) {
            const node = stack.pop();
            if (node.nodeType === Node.TEXT_NODE) continue;
            if (node.nodeType === Node.ELEMENT_NODE) {
                if (node.tagName === 'BR') continue;
                if (getDiffElementRole(node) && !shouldSkipElement(node)) {
                    for (let k = 0; k < node.childNodes.length; k++) {
                        stack.push(node.childNodes[k]);
                    }
                    continue;
                }
            }
            return false;
        }
        return true;
    }

    function appendDiffNodeSegments(node, inheritedRole, groupId, topElement, state) {
        if (node.nodeType === Node.TEXT_NODE) {
            const text = node.nodeValue || '';
            if (!text.length) return;
            state.segments.push({
                isBr: false,
                text: text,
                diff: inheritedRole,
                groupId: groupId,
                groupElement: topElement,
                start: state.offset,
                end: state.offset + text.length
            });
            state.combined += text;
            state.offset += text.length;
        } else if (node.nodeType === Node.ELEMENT_NODE && node.tagName === 'BR') {
            state.segments.push({
                isBr: true,
                text: '\n',
                diff: inheritedRole,
                groupId: groupId,
                groupElement: topElement,
                start: state.offset,
                end: state.offset + 1
            });
            state.combined += '\n';
            state.offset += 1;
        } else if (node.nodeType === Node.ELEMENT_NODE) {
            const role = getDiffElementRole(node);
            const effectiveRole = role === 'char' ? inheritedRole : (role || inheritedRole);
            const children = node.childNodes;
            for (let i = 0; i < children.length; i++) {
                appendDiffNodeSegments(children[i], effectiveRole, groupId, topElement, state);
            }
        }
    }

    /**
     * Slices the original segment list for [start, end) so text, <br>, and inline diff
     * elements are preserved cleanly for both un-matched regions and lossless toggle restore.
     */
    function sliceRunRange(segments, start, end, preservedDiffEls) {
        const frag = document.createDocumentFragment();
        let rawWithBr = '';
        const emittedGroups = new Set();

        for (let i = 0; i < segments.length; i++) {
            const seg = segments[i];
            if (seg.end <= start || seg.start >= end) {
                continue;
            }
            if (seg.groupId !== null && seg.groupElement) {
                if (!emittedGroups.has(seg.groupId)) {
                    emittedGroups.add(seg.groupId);
                    const clone = seg.groupElement.cloneNode(true);
                    frag.appendChild(clone);
                    if (preservedDiffEls) {
                        preservedDiffEls.push(clone);
                    }
                }
                if (seg.isBr) {
                    rawWithBr += BR_SENTINEL;
                } else {
                    const sliceFrom = Math.max(0, start - seg.start);
                    const sliceTo = Math.min(seg.text.length, end - seg.start);
                    if (sliceTo > sliceFrom) {
                        rawWithBr += seg.text.slice(sliceFrom, sliceTo).split(BR_SENTINEL).join('');
                    }
                }
                continue;
            }
            if (seg.isBr) {
                frag.appendChild(document.createElement('br'));
                rawWithBr += BR_SENTINEL;
            } else {
                const sliceFrom = Math.max(0, start - seg.start);
                const sliceTo = Math.min(seg.text.length, end - seg.start);
                if (sliceTo > sliceFrom) {
                    const part = seg.text.slice(sliceFrom, sliceTo);
                    frag.appendChild(document.createTextNode(part));
                    rawWithBr += part.split(BR_SENTINEL).join('');
                }
            }
        }

        return { fragment: frag, rawWithBr: rawWithBr };
    }

    function buildInnerDiffSegments(segments, innerStart, innerEnd) {
        const merged = [];
        for (let i = 0; i < segments.length; i++) {
            const seg = segments[i];
            if (seg.end <= innerStart || seg.start >= innerEnd) continue;
            const sliceFrom = Math.max(0, innerStart - seg.start);
            const sliceTo = Math.min(seg.text.length, innerEnd - seg.start);
            if (sliceTo <= sliceFrom) continue;
            const text = seg.text.slice(sliceFrom, sliceTo);
            const role = (seg.diff === 'del' || seg.diff === 'ins') ? seg.diff : null;
            if (merged.length > 0 && merged[merged.length - 1].diff === role) {
                merged[merged.length - 1].text += text;
            } else {
                merged.push({ text: text, diff: role });
            }
        }
        return merged;
    }

    function isSiblingRunChild(child) {
        return (
            child.nodeType === Node.TEXT_NODE ||
            (child.nodeType === Node.ELEMENT_NODE &&
                (child.tagName === 'BR' || isFlattenableDiffElement(child)))
        );
    }

    function hasNonEmptyMathInner(rawText, start, end) {
        return rawText.slice(start + 2, end - 2).split(BR_SENTINEL).join('').trim().length > 0;
    }

    function analyzeSiblingRun(runNodes) {
        const state = { segments: [], combined: '', offset: 0 };
        let nextGroupId = 1;
        const groupBounds = new Map();

        for (let i = 0; i < runNodes.length; i++) {
            const node = runNodes[i];
            if (node.nodeType === Node.TEXT_NODE) {
                const text = node.nodeValue || '';
                if (!text.length) continue;
                state.segments.push({
                    isBr: false,
                    text: text,
                    diff: null,
                    groupId: null,
                    groupElement: null,
                    start: state.offset,
                    end: state.offset + text.length
                });
                state.combined += text;
                state.offset += text.length;
            } else if (node.nodeType === Node.ELEMENT_NODE && node.tagName === 'BR') {
                state.segments.push({
                    isBr: true,
                    text: '\n',
                    diff: null,
                    groupId: null,
                    groupElement: null,
                    start: state.offset,
                    end: state.offset + 1
                });
                state.combined += '\n';
                state.offset += 1;
            } else if (node.nodeType === Node.ELEMENT_NODE && getDiffElementRole(node)) {
                const gid = nextGroupId++;
                const gStart = state.offset;
                const role = getDiffElementRole(node);
                appendDiffNodeSegments(node, role === 'char' ? null : role, gid, node, state);
                groupBounds.set(gid, { start: gStart, end: state.offset, element: node });
            }
        }

        const segments = state.segments;
        const combined = state.combined;
        if (!combined || (!combined.includes('\\(') && !combined.includes('\\['))) {
            return { segments: segments, combined: combined, matches: [] };
        }

        const rawMatches = findMathMatches(combined);
        const matches = [];
        for (let m = 0; m < rawMatches.length; m++) {
            const match = rawMatches[m];
            const groups = new Set();
            let hasPlainSegment = false;
            let partialBoundary = false;

            for (let s = 0; s < segments.length; s++) {
                const seg = segments[s];
                if (seg.end <= match.start || seg.start >= match.end) continue;
                if (seg.groupId === null) {
                    hasPlainSegment = true;
                } else {
                    groups.add(seg.groupId);
                    const bounds = groupBounds.get(seg.groupId);
                    if (bounds && (bounds.start < match.start || bounds.end > match.end)) {
                        partialBoundary = true;
                        break;
                    }
                }
            }

            // Matches contained entirely inside a single diff wrapper render recursively inside that
            // wrapper so `.diff-highlight-* .mathjax-rendered` styles the whole formula atomically.
            if (partialBoundary || (!hasPlainSegment && groups.size === 1)) {
                continue;
            }
            matches.push(match);
        }

        return { segments: segments, combined: combined, matches: matches };
    }

    function processSiblingRun(parentEl, runNodes, jobs) {
        const analysis = analyzeSiblingRun(runNodes);
        const segments = analysis.segments;
        const combined = analysis.combined;
        const matches = analysis.matches;

        if (!matches.length) {
            for (let r = 0; r < runNodes.length; r++) {
                if (runNodes[r].nodeType === Node.ELEMENT_NODE && getDiffElementRole(runNodes[r])) {
                    collectRunsInElement(runNodes[r], jobs);
                }
            }
            return;
        }

        const frag = document.createDocumentFragment();
        const preservedDiffEls = [];
        let cursor = 0;

        for (let m = 0; m < matches.length; m++) {
            const match = matches[m];
            if (match.start > cursor) {
                const before = sliceRunRange(segments, cursor, match.start, preservedDiffEls);
                frag.appendChild(before.fragment);
            }

            const matchedSlice = sliceRunRange(segments, match.start, match.end, null);
            const rawCombined = combined.slice(match.start, match.end);
            const innerDiffSegs = buildInnerDiffSegments(segments, match.start + 2, match.end - 2);
            const hasDiffHighlight = innerDiffSegs.some(function (s) {
                return s.diff === 'del' || s.diff === 'ins';
            });
            const tex = rawCombined.slice(2, rawCombined.length - 2).trim();

            const wrapper = document.createElement('span');
            wrapper.className = 'mathjax-rendered ' + (match.display ? 'mathjax-display' : 'mathjax-inline');
            wrapper.setAttribute('data-mathjax-raw', matchedSlice.rawWithBr);
            wrapper.setAttribute('data-mathjax-display', match.display ? 'true' : 'false');
            if (hasDiffHighlight) {
                rawFragmentStore.set(wrapper, matchedSlice.fragment);
            }
            wrapper.textContent = rawCombined;
            frag.appendChild(wrapper);

            if (tex) {
                const job = {
                    element: wrapper,
                    tex: tex,
                    display: match.display
                };
                if (hasDiffHighlight) {
                    job.diffSegments = innerDiffSegs;
                }
                jobs.push(job);
            }

            cursor = match.end;
        }

        if (cursor < combined.length) {
            const after = sliceRunRange(segments, cursor, combined.length, preservedDiffEls);
            frag.appendChild(after.fragment);
        }

        const insertBeforeNode = runNodes[runNodes.length - 1].nextSibling;
        for (let r = 0; r < runNodes.length; r++) {
            parentEl.removeChild(runNodes[r]);
        }
        parentEl.insertBefore(frag, insertBeforeNode);

        for (let p = 0; p < preservedDiffEls.length; p++) {
            collectRunsInElement(preservedDiffEls[p], jobs);
        }
    }

    function collectRunsInElement(el, jobs) {
        if (shouldSkipElement(el)) return;

        // Snapshot childNodes before mutating runs
        const children = Array.from(el.childNodes);
        let run = [];

        for (let i = 0; i < children.length; i++) {
            const child = children[i];
            if (isSiblingRunChild(child)) {
                run.push(child);
            } else {
                if (run.length > 0) {
                    processSiblingRun(el, run, jobs);
                    run = [];
                }
                if (child.nodeType === Node.ELEMENT_NODE) {
                    collectRunsInElement(child, jobs);
                }
            }
        }
        if (run.length > 0) {
            processSiblingRun(el, run, jobs);
        }
    }

    function runHasRenderableMath(runNodes) {
        const analysis = analyzeSiblingRun(runNodes);
        for (let m = 0; m < analysis.matches.length; m++) {
            const match = analysis.matches[m];
            if (hasNonEmptyMathInner(analysis.combined, match.start, match.end)) {
                return true;
            }
        }
        for (let r = 0; r < runNodes.length; r++) {
            if (runNodes[r].nodeType === Node.ELEMENT_NODE && getDiffElementRole(runNodes[r])) {
                if (elementHasRenderableMath(runNodes[r])) {
                    return true;
                }
            }
        }
        return false;
    }

    function elementHasRenderableMath(el) {
        if (shouldSkipElement(el)) return false;

        const children = el.childNodes;
        let run = [];

        for (let i = 0; i < children.length; i++) {
            const child = children[i];
            if (isSiblingRunChild(child)) {
                run.push(child);
            } else {
                if (run.length > 0) {
                    if (runHasRenderableMath(run)) return true;
                    run = [];
                }
                if (child.nodeType === Node.ELEMENT_NODE && elementHasRenderableMath(child)) {
                    return true;
                }
            }
        }
        return run.length > 0 && runHasRenderableMath(run);
    }

    function hasSkippedAncestor(el, stopAt) {
        let cur = el.parentElement;
        while (cur && cur !== stopAt) {
            if (shouldSkipElement(cur)) {
                return true;
            }
            cur = cur.parentElement;
        }
        return false;
    }

    function hasRenderedFormula(container) {
        const rendered = container.querySelectorAll('.mathjax-rendered[data-mathjax-raw]');
        for (let i = 0; i < rendered.length; i++) {
            const span = rendered[i];
            const raw = span.getAttribute('data-mathjax-raw') || '';
            if (hasNonEmptyMathInner(raw, 0, raw.length) && !hasSkippedAncestor(span, container)) {
                return true;
            }
        }
        return false;
    }

    function isExcludedPageContainer(el) {
        if (!el || el.nodeType !== Node.ELEMENT_NODE) return true;
        if (el.classList.contains('mathjax-editor-preview-body')) return true;
        if (typeof el.closest === 'function' && el.closest('.edit-all-fields-panel, .reviews-detail[hidden]')) {
            return true;
        }
        return hasSkippedAncestor(el, null);
    }

    function containerHasRenderableMath(container) {
        if (!container || shouldSkipElement(container)) {
            return false;
        }
        if (hasRenderedFormula(container)) {
            return true;
        }
        const rawText = container.textContent || '';
        if (!rawText.includes('\\(') && !rawText.includes('\\[')) {
            return false;
        }
        return elementHasRenderableMath(container);
    }

    function pageHasRenderableMath() {
        const containers = document.querySelectorAll(CONTAINER_SELECTOR);
        for (let i = 0; i < containers.length; i++) {
            const el = containers[i];
            if (!isExcludedPageContainer(el) && containerHasRenderableMath(el)) {
                return true;
            }
        }
        return false;
    }

    function hasRenderableMath(rootEl) {
        if (rootEl && rootEl.nodeType === Node.ELEMENT_NODE) {
            return elementHasRenderableMath(rootEl);
        }
        return pageHasRenderableMath();
    }

    function updateToggleVisibility() {
        const buttons = document.querySelectorAll('.mathjax-toggle-btn');
        let hasMath = null;
        for (let i = 0; i < buttons.length; i++) {
            const btn = buttons[i];
            if (btn.closest && btn.closest('.edit-all-fields-panel')) {
                continue;
            }
            if (hasMath === null) {
                hasMath = pageHasRenderableMath();
            }
            btn.hidden = !hasMath;
        }
    }

    function restoreSpanElement(span, parentsToNormalize) {
        if (!span || !span.parentNode) return;
        const parent = span.parentNode;
        const storedFrag = rawFragmentStore.get(span);
        if (storedFrag) {
            const restored = storedFrag.cloneNode(true);
            rawFragmentStore.delete(span);
            parent.replaceChild(restored, span);
            if (parentsToNormalize) {
                parentsToNormalize.add(parent);
            }
            return;
        }
        const raw = span.getAttribute('data-mathjax-raw') || '';
        const parts = raw.includes(BR_SENTINEL)
            ? raw.split(BR_SENTINEL)
            : raw.split(/<br\s*\/?>/i);
        const frag = document.createDocumentFragment();

        for (let p = 0; p < parts.length; p++) {
            if (parts[p]) {
                frag.appendChild(document.createTextNode(parts[p]));
            }
            if (p < parts.length - 1) {
                frag.appendChild(document.createElement('br'));
            }
        }

        parent.replaceChild(frag, span);
        if (parentsToNormalize) {
            parentsToNormalize.add(parent);
        }
    }

    function applySvgResult(spanEl, result) {
        if (!spanEl || !spanEl.isConnected) return;
        if (!result || !result.ok || !result.svgEl) {
            spanEl.classList.add('mathjax-error');
            if (result && result.error) {
                spanEl.setAttribute('title', String(result.error));
            }
            return;
        }
        spanEl.textContent = '';
        spanEl.classList.remove('mathjax-error');
        spanEl.removeAttribute('title');
        const clonedSvg = result.svgEl.cloneNode(true);
        if (Array.isArray(result.styleBindings)) {
            for (let b = 0; b < result.styleBindings.length; b++) {
                const binding = result.styleBindings[b];
                let target = clonedSvg;
                for (let p = 0; p < binding.path.length && target; p++) {
                    target = target.children[binding.path[p]];
                }
                if (target && target.style && typeof target.style.setProperty === 'function') {
                    for (let d = 0; d < binding.decls.length; d++) {
                        target.style.setProperty(binding.decls[d].prop, binding.decls[d].val);
                    }
                }
            }
        }
        spanEl.appendChild(clonedSvg);
    }

    /**
     * Scans .note-content-display / .mathjax-editor-preview-body elements within rootEl,
     * replaces closed TeX blocks with placeholder spans, and populates them asynchronously
     * using the sandboxed MathJax worker + strict SVG allowlist sanitizer.
     * @param {HTMLElement|Document} rootEl
     * @returns {Promise<void>}
     */
    function renderInElement(rootEl) {
        if (!isEnabled() || !rootEl) {
            return Promise.resolve();
        }

        const containers = [];
        if (
            rootEl.nodeType === Node.ELEMENT_NODE &&
            typeof rootEl.matches === 'function' &&
            rootEl.matches(CONTAINER_SELECTOR)
        ) {
            containers.push(rootEl);
        } else if (typeof rootEl.querySelectorAll === 'function') {
            const found = rootEl.querySelectorAll(CONTAINER_SELECTOR);
            for (let i = 0; i < found.length; i++) {
                containers.push(found[i]);
            }
        }

        if (!containers.length) {
            return Promise.resolve();
        }

        const jobs = [];
        for (let c = 0; c < containers.length; c++) {
            collectRunsInElement(containers[c], jobs);
        }

        if (!jobs.length) {
            return Promise.resolve();
        }

        const uncachedByKey = new Map();
        const itemsToRequest = [];

        for (let j = 0; j < jobs.length; j++) {
            const job = jobs[j];
            const cacheKey = (job.display ? 'D:' : 'I:') + (job.diffSegments ? 'DIFF:' + JSON.stringify(job.diffSegments) : job.tex);
            job.cacheKey = cacheKey;

            if (svgCache.has(cacheKey)) {
                applySvgResult(job.element, svgCache.get(cacheKey));
            } else if (!uncachedByKey.has(cacheKey)) {
                const itemId = 'expr_' + itemsToRequest.length;
                uncachedByKey.set(cacheKey, itemId);
                const reqItem = {
                    id: itemId,
                    tex: job.tex,
                    display: job.display,
                    cacheKey: cacheKey
                };
                if (job.diffSegments) {
                    reqItem.diffSegments = job.diffSegments;
                }
                itemsToRequest.push(reqItem);
            }
        }

        if (!itemsToRequest.length) {
            return Promise.resolve();
        }

        const payload = itemsToRequest.map(function (item) {
            const out = { id: item.id, tex: item.tex, display: item.display };
            if (item.diffSegments) {
                out.diffSegments = item.diffSegments;
            }
            return out;
        });

        return sendRenderBatch(payload).then(function (results) {
            if (!isEnabled()) {
                return;
            }
            const parentsToNormalize = new Set();
            if (!results) {
                console.warn('MathJax worker unavailable or timed out; leaving raw TeX for retry.');
                for (let j = 0; j < jobs.length; j++) {
                    const job = jobs[j];
                    if (uncachedByKey.has(job.cacheKey) && job.element && job.element.isConnected) {
                        restoreSpanElement(job.element, parentsToNormalize);
                    }
                }
                parentsToNormalize.forEach(function (parent) {
                    parent.normalize();
                });
                return;
            }

            const transientKeys = new Set();
            for (let i = 0; i < itemsToRequest.length; i++) {
                const req = itemsToRequest[i];
                const rawRes = results[req.id];
                if (!rawRes || (!rawRes.ok && rawRes.error === 'MathJax tex2svg unavailable')) {
                    transientKeys.add(req.cacheKey);
                    continue;
                }
                let cachedEntry;
                if (rawRes.ok && rawRes.svg) {
                    const cleanResult = sanitizeSvgMarkup(rawRes.svg);
                    if (cleanResult) {
                        cachedEntry = {
                            ok: true,
                            svgEl: cleanResult.svgEl,
                            styleBindings: cleanResult.styleBindings
                        };
                    } else {
                        cachedEntry = { ok: false, error: 'Sanitizer rejected SVG output' };
                    }
                } else {
                    cachedEntry = {
                        ok: false,
                        error: rawRes.error ? String(rawRes.error) : 'MathJax render error'
                    };
                }

                if (svgCache.size >= MAX_CACHE_ENTRIES) {
                    svgCache.clear();
                }
                svgCache.set(req.cacheKey, cachedEntry);
            }

            for (let j = 0; j < jobs.length; j++) {
                const job = jobs[j];
                if (transientKeys.has(job.cacheKey)) {
                    if (job.element && job.element.isConnected) {
                        restoreSpanElement(job.element, parentsToNormalize);
                    }
                } else if (svgCache.has(job.cacheKey)) {
                    applySvgResult(job.element, svgCache.get(job.cacheKey));
                }
            }
            parentsToNormalize.forEach(function (parent) {
                parent.normalize();
            });
        });
    }

    function renderAll(rootEl) {
        return renderInElement(rootEl || document);
    }

    /**
     * Losslessly restores .mathjax-rendered spans back to original TextNode + <br> nodes
     * without touching innerHTML (preserving lazy-loaded images and diff wrappers).
     * @param {HTMLElement|Document} rootEl
     */
    function restoreInElement(rootEl) {
        const scope = rootEl || document;
        if (!scope || typeof scope.querySelectorAll !== 'function') return;

        const rendered = scope.querySelectorAll('.mathjax-rendered[data-mathjax-raw]');
        const parentsToNormalize = new Set();

        for (let i = 0; i < rendered.length; i++) {
            restoreSpanElement(rendered[i], parentsToNormalize);
        }

        parentsToNormalize.forEach(function (parent) {
            parent.normalize();
        });
    }

    function restoreAll(rootEl) {
        restoreInElement(rootEl || document);
    }

    // Wire up delegated toggle click handler and initial state on DOMContentLoaded
    document.addEventListener('DOMContentLoaded', function () {
        updateToggleButtons(isEnabled());

        document.addEventListener('click', function (event) {
            const btn = event.target && event.target.closest ? event.target.closest('.mathjax-toggle-btn') : null;
            if (!btn) return;
            event.preventDefault();
            toggle();
        });

        if (isEnabled()) {
            renderAll(document);
        }
        updateToggleVisibility();
    });

    return {
        STORAGE_KEY: STORAGE_KEY,
        isEnabled: isEnabled,
        setEnabled: setEnabled,
        toggle: toggle,
        containsMathDelimiters: containsMathDelimiters,
        hasRenderableMath: hasRenderableMath,
        renderInElement: renderInElement,
        renderAll: renderAll,
        restoreInElement: restoreInElement,
        restoreAll: restoreAll,
        updateToggleButtons: updateToggleButtons,
        updateToggleVisibility: updateToggleVisibility
    };
})();
