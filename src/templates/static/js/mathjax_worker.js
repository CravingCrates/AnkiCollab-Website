/**
 * Sandboxed MathJax v3 Worker.
 * Runs inside <iframe sandbox="allow-scripts" src="/static/mathjax_sandbox.html">
 * with an opaque origin, no cookies, and no host DOM access.
 */
(function () {
    'use strict';

    let isReady = false;
    let pinnedParentOrigin = null;
    let pinnedParentOriginStr = null;
    const pendingBatches = [];
    const serializer = typeof XMLSerializer !== 'undefined' ? new XMLSerializer() : null;

    function getHostTargetOrigin() {
        if (pinnedParentOriginStr && pinnedParentOriginStr !== 'null') {
            return pinnedParentOriginStr;
        }
        try {
            if (document.referrer) {
                const refOrigin = new URL(document.referrer).origin;
                if (refOrigin && refOrigin !== 'null') {
                    return refOrigin;
                }
            }
        } catch (_) {
            // Fall through to wildcard when referrer is unavailable
        }
        return '*';
    }

    /**
     * Rewrites Anki Cloze deletions ({{c1::answer}} / {{c1::answer::hint}}) inside TeX
     * into a highlighted mathematical group so MathJax renders nested formulas cleanly.
     * @param {string} tex
     * @returns {string}
     */
    function preprocessAnkiCloze(tex) {
        if (typeof tex !== 'string' || (!tex.includes('{{c') && !tex.includes('{{C'))) {
            return tex;
        }
        let out = '';
        let i = 0;
        const len = tex.length;

        while (i < len) {
            if (
                tex[i] === '{' &&
                tex[i + 1] === '{' &&
                (tex[i + 2] === 'c' || tex[i + 2] === 'C')
            ) {
                let k = i + 3;
                while (k < len && tex[k] >= '0' && tex[k] <= '9') {
                    k += 1;
                }
                if (k > i + 3 && tex[k] === ':' && tex[k + 1] === ':') {
                    const num = tex.slice(i + 3, k);
                    const contentStart = k + 2;
                    let j = contentStart;
                    let braceDepth = 0;
                    let hintSplit = -1;
                    let closeIdx = -1;

                    while (j + 1 < len) {
                        if (tex[j] === '\\') {
                            j += 2;
                            continue;
                        }
                        if (tex[j] === '{') {
                            braceDepth += 1;
                            j += 1;
                            continue;
                        }
                        if (tex[j] === '}') {
                            if (braceDepth > 0) {
                                braceDepth -= 1;
                                j += 1;
                                continue;
                            }
                            if (tex[j + 1] === '}') {
                                closeIdx = j;
                                break;
                            }
                        }
                        if (braceDepth === 0 && hintSplit === -1 && tex[j] === ':' && tex[j + 1] === ':') {
                            hintSplit = j;
                            j += 2;
                            continue;
                        }
                        j += 1;
                    }

                    if (closeIdx !== -1) {
                        const answer = hintSplit === -1
                            ? tex.slice(contentStart, closeIdx)
                            : tex.slice(contentStart, hintSplit);
                        out += '{\\color{#2563eb}{\\mathtt{[c' + num + ':}\\,{' + answer + '}\\,\\mathtt{]}}}';
                        i = closeIdx + 2;
                        continue;
                    }
                }
            }
            out += tex[i];
            i += 1;
        }
        return out;
    }

    /**
     * Validates that incoming postMessage events come exclusively from the host window
     * using structured Origin parsing when supported by the browser.
     * @param {MessageEvent} event
     * @returns {boolean}
     */
    function isValidHostEvent(event) {
        if (!window.parent || window.parent === window) {
            return false;
        }
        if (event.source !== window.parent) {
            return false;
        }
        if (typeof Origin !== 'undefined' && typeof Origin.from === 'function') {
            try {
                const senderOrigin = Origin.from(event);
                if (senderOrigin.opaque) {
                    return false;
                }
                if (!pinnedParentOrigin) {
                    pinnedParentOrigin = senderOrigin;
                } else if (
                    typeof pinnedParentOrigin.isSameOrigin === 'function' &&
                    !pinnedParentOrigin.isSameOrigin(senderOrigin)
                ) {
                    return false;
                }
            } catch (_) {
                return false;
            }
        } else {
            if (typeof event.origin !== 'string' || !event.origin || event.origin === 'null') {
                return false;
            }
            if (!pinnedParentOriginStr) {
                pinnedParentOriginStr = event.origin;
            } else if (pinnedParentOriginStr !== event.origin) {
                return false;
            }
            return true;
        }
        if (typeof event.origin === 'string' && event.origin && event.origin !== 'null' && !pinnedParentOriginStr) {
            pinnedParentOriginStr = event.origin;
        }
        return true;
    }

    function wrapDiffSegment(text, role, useBbox) {
        if (!text) return '';
        if (!role || !text.trim()) return text;
        const fg = role === 'del' ? '#991b1b' : '#166534';
        const bg = role === 'del' ? '#fecaca' : '#bbf7d0';
        const wrap = function (inner) {
            if (!inner) return '';
            return useBbox
                ? '{\\bbox[' + bg + ',2px]{\\textcolor{' + fg + '}{' + inner + '}}}'
                : '{\\textcolor{' + fg + '}{' + inner + '}}';
        };
        if ((text[0] === '^' || text[0] === '_') && text.length > 1) {
            const rest = text.slice(1);
            if (rest.trim()) {
                return text[0] + wrap(rest);
            }
        }
        return wrap(text);
    }

    function buildHighlightedTex(diffSegments, useBbox) {
        if (!Array.isArray(diffSegments) || !diffSegments.length) {
            return '';
        }
        let out = '';
        for (let i = 0; i < diffSegments.length; i++) {
            const seg = diffSegments[i];
            if (!seg || typeof seg.text !== 'string') continue;
            const role = (seg.diff === 'del' || seg.diff === 'ins') ? seg.diff : null;
            out += wrapDiffSegment(seg.text, role, useBbox);
        }
        return out.trim();
    }

    /**
     * Converts a single TeX expression (optionally with intra-formula diff segments)
     * into serialized self-contained SVG markup.
     * @param {string} tex
     * @param {boolean} display
     * @param {Array<{text: string, diff: string|null}>|null} [diffSegments]
     * @returns {{ ok: boolean, svg?: string, error?: string }}
     */
    function renderTexToSvg(tex, display, diffSegments) {
        if (!window.MathJax || typeof window.MathJax.tex2svg !== 'function') {
            return { ok: false, error: 'MathJax tex2svg unavailable' };
        }

        const attemptRender = function (sourceTex) {
            try {
                const container = window.MathJax.tex2svg(sourceTex, { display: Boolean(display) });
                const errNode = container
                    ? container.querySelector('[data-mjx-error], [data-mml-node="merror"], merror')
                    : null;
                if (errNode) {
                    const errMsg =
                        errNode.getAttribute('data-mjx-error') ||
                        errNode.getAttribute('title') ||
                        'MathJax syntax error';
                    return { ok: false, error: errMsg };
                }
                const svg = container ? container.querySelector('svg') : null;
                if (!svg) {
                    return { ok: false, error: 'Missing SVG output' };
                }
                svg.removeAttribute('focusable');
                svg.setAttribute('aria-hidden', 'true');
                const transferStyleAttr = function (el) {
                    if (el && typeof el.hasAttribute === 'function' && el.hasAttribute('style')) {
                        const styleVal = el.getAttribute('style');
                        if (styleVal && !el.hasAttribute('data-mjx-style')) {
                            el.setAttribute('data-mjx-style', styleVal);
                        }
                        el.removeAttribute('style');
                    }
                };
                transferStyleAttr(svg);
                if (typeof svg.querySelectorAll === 'function') {
                    const styled = svg.querySelectorAll('[style]');
                    for (let i = 0; i < styled.length; i++) {
                        transferStyleAttr(styled[i]);
                    }
                }
                const svgMarkup = serializer ? serializer.serializeToString(svg) : svg.outerHTML;
                return { ok: true, svg: svgMarkup };
            } catch (err) {
                return { ok: false, error: err && err.message ? String(err.message) : 'Render failed' };
            }
        };

        const renderWithClozeFallback = function (sourceTex) {
            const preprocessed = preprocessAnkiCloze(sourceTex);
            const firstResult = attemptRender(preprocessed);
            if (!firstResult.ok && preprocessed !== sourceTex) {
                const fallbackResult = attemptRender(sourceTex);
                if (fallbackResult.ok) {
                    return fallbackResult;
                }
            }
            return firstResult;
        };

        if (Array.isArray(diffSegments) && diffSegments.length > 0) {
            const bboxTex = buildHighlightedTex(diffSegments, true);
            if (bboxTex) {
                const bboxRes = renderWithClozeFallback(bboxTex);
                if (bboxRes.ok) {
                    return bboxRes;
                }
            }
            const colorTex = buildHighlightedTex(diffSegments, false);
            if (colorTex) {
                const colorRes = renderWithClozeFallback(colorTex);
                if (colorRes.ok) {
                    return colorRes;
                }
            }
        }

        return renderWithClozeFallback(tex);
    }

    function processBatch(data) {
        const items = Array.isArray(data.items) ? data.items : [];
        const results = {};
        for (let i = 0; i < items.length; i++) {
            const item = items[i];
            if (!item || typeof item.id !== 'string') continue;
            const tex = typeof item.tex === 'string' ? item.tex : '';
            const diffSegments = Array.isArray(item.diffSegments) ? item.diffSegments : null;
            results[item.id] = renderTexToSvg(tex, Boolean(item.display), diffSegments);
        }

        if (window.parent && window.parent !== window) {
            window.parent.postMessage(
                {
                    type: 'mathjax-render-result',
                    requestId: data.requestId,
                    results: results
                },
                getHostTargetOrigin()
            );
        }
    }

    function flushPendingQueue() {
        while (pendingBatches.length > 0) {
            const next = pendingBatches.shift();
            processBatch(next);
        }
    }

    window.MathJax = {
        options: {
            enableMenu: false,
            enableAssistiveMml: false
        },
        tex: {
            packages: { '[+]': ['physics'], '[-]': ['html', 'action'] },
            maxMacros: 1000,
            maxBuffer: 10240
        },
        svg: {
            fontCache: 'none'
        },
        startup: {
            typeset: false,
            ready: function () {
                window.MathJax.startup.defaultReady();
                const adaptor = window.MathJax.startup.adaptor;
                if (adaptor && typeof adaptor.setAttribute === 'function') {
                    const origSetAttribute = adaptor.setAttribute.bind(adaptor);
                    adaptor.setAttribute = function (node, name, value, ns) {
                        if (!ns && name === 'style' && node && typeof node.setAttribute === 'function') {
                            return node.setAttribute('data-mjx-style', String(value));
                        }
                        return origSetAttribute(node, name, value, ns);
                    };
                }
                isReady = true;
                if (window.parent && window.parent !== window) {
                    window.parent.postMessage({ type: 'mathjax-worker-ready' }, getHostTargetOrigin());
                }
                flushPendingQueue();
            }
        }
    };

    window.addEventListener('message', function (event) {
        if (!isValidHostEvent(event)) return;
        const data = event.data;
        if (!data || typeof data !== 'object') return;
        if (data.type !== 'mathjax-render-batch' || typeof data.requestId !== 'string') return;

        if (!isReady) {
            pendingBatches.push(data);
            return;
        }
        processBatch(data);
    });
})();
