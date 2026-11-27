/**
 * webhooks.js - Deck Discord Webhooks management UI
 * Uses native <dialog> modals, deferred (:user-invalid) validation with ARIA sync,
 * and strict textContent updates without HTML interpolation.
 */
document.addEventListener('DOMContentLoaded', function() {
    var pageEl = document.getElementById('webhooks-page');
    if (!pageEl) return;

    var deckHash = pageEl.dataset.deckHash || '';
    var maxWebhooks = parseInt(pageEl.dataset.maxWebhooks || '5', 10);

    var createDialog = document.getElementById('wh-create-dialog');
    var deleteDialog = document.getElementById('wh-delete-dialog');
    var createForm = document.getElementById('wh-create-form');
    var createErrorEl = document.getElementById('wh-create-error');
    var feedbackEl = document.getElementById('wh-feedback');
    var pendingDeleteId = null;
    var pendingDeleteCard = null;
    var feedbackTimer = null;

    // Polite live region for field validation errors (separate from #wh-feedback status).
    var announcer = document.createElement('div');
    announcer.setAttribute('aria-live', 'polite');
    announcer.classList.add('visually-hidden');
    document.body.append(announcer);

    var announceTimer = null;
    var announcedField = null;
    var flagged = new WeakSet();
    var announced = new WeakSet();
    var dirtyFallbackState = new WeakMap();
    var supportsUserInvalid = typeof CSS !== 'undefined' && CSS.supports && CSS.supports('selector(:user-invalid)');

    function spokenText(node) {
        var copy = node.cloneNode(true);
        copy.querySelectorAll('[aria-hidden="true"]').forEach(function(n) { n.remove(); });
        copy.querySelectorAll('[role="img"][aria-label]').forEach(function(n) {
            n.replaceWith(n.getAttribute('aria-label') + ': ');
        });
        return (copy.textContent || '').replace(/\s+/g, ' ').trim();
    }

    function linkError(el, shown) {
        var errorId = el.dataset.error;
        if (!errorId) return;
        var ids = (el.getAttribute('aria-describedby') || '')
            .split(/\s+/)
            .filter(function(id) { return id && id !== errorId; });
        if (shown) ids.push(errorId);
        if (ids.length) {
            el.setAttribute('aria-describedby', ids.join(' '));
        } else {
            el.removeAttribute('aria-describedby');
        }
    }

    function announceFieldError(el) {
        var errorNode = el.dataset.error ? document.getElementById(el.dataset.error) : null;
        if (!errorNode) return;
        var label = (el.labels && el.labels[0]) ? spokenText(el.labels[0]) + ': ' : '';
        clearTimeout(announceTimer);
        announcer.textContent = '';
        announcedField = el;
        announceTimer = setTimeout(function() {
            announcer.textContent = label + spokenText(errorNode);
        }, 500);
    }

    function syncAria(el, shouldAnnounce) {
        if (!el || !el.matches || !el.matches('input, select, textarea')) return;
        var invalid = supportsUserInvalid
            ? el.matches(':user-invalid')
            : el.classList.contains('user-invalid-fallback');
        if (invalid) {
            if (shouldAnnounce && !announced.has(el)) {
                announceFieldError(el);
                announced.add(el);
            }
            el.setAttribute('aria-invalid', 'true');
            linkError(el, true);
            flagged.add(el);
        } else if (flagged.has(el) || el.getAttribute('aria-invalid') !== 'true') {
            el.setAttribute('aria-invalid', 'false');
            linkError(el, false);
            flagged.delete(el);
            announced.delete(el);
            if (announcedField === el) {
                clearTimeout(announceTimer);
                announcer.textContent = '';
            }
        }
    }

    function initValidation(root) {
        root.querySelectorAll('input, select, textarea').forEach(function(el) {
            syncAria(el, false);
        });

        root.addEventListener('blur', function(e) {
            if (!supportsUserInvalid && e.target.checkValidity) {
                var st = dirtyFallbackState.get(e.target) || { hasInteracted: false, hasBlurred: false };
                st.hasBlurred = true;
                dirtyFallbackState.set(e.target, st);
                if (st.hasInteracted) {
                    var ok = e.target.validity.valid;
                    e.target.classList.toggle('user-invalid-fallback', !ok);
                }
            }
            syncAria(e.target, true);
        }, true);

        root.addEventListener('input', function(e) {
            if (!supportsUserInvalid && e.target.checkValidity) {
                var st = dirtyFallbackState.get(e.target) || { hasInteracted: false, hasBlurred: false };
                st.hasInteracted = true;
                dirtyFallbackState.set(e.target, st);
                if (st.hasBlurred) {
                    var ok = e.target.validity.valid;
                    e.target.classList.toggle('user-invalid-fallback', !ok);
                }
            }
            if (e.target.hasAttribute('aria-invalid')) {
                syncAria(e.target, false);
            }
        });

        root.addEventListener('change', function(e) {
            syncAria(e.target, true);
        }, true);

        root.addEventListener('invalid', function(e) {
            if (!supportsUserInvalid) {
                dirtyFallbackState.set(e.target, { hasInteracted: true, hasBlurred: true });
                e.target.classList.add('user-invalid-fallback');
            }
            announced.add(e.target);
            syncAria(e.target, false);
        }, true);

        root.addEventListener('reset', function(e) {
            setTimeout(function() {
                if (!e.target || !e.target.elements) return;
                Array.prototype.forEach.call(e.target.elements, function(ctrl) {
                    dirtyFallbackState.delete(ctrl);
                    ctrl.classList.remove('user-invalid-fallback');
                    syncAria(ctrl, false);
                });
            });
        }, true);
    }

    initValidation(pageEl);
    if (createDialog) initValidation(createDialog);

    // Light-dismiss fallback for browsers without <dialog closedby="any"> (e.g. Safari).
    if (!('closedBy' in HTMLDialogElement.prototype)) {
        var outsideDialog = function(e, dlg) {
            var r = dlg.getBoundingClientRect();
            return e.clientX < r.left || e.clientX > r.right || e.clientY < r.top || e.clientY > r.bottom;
        };
        document.querySelectorAll('dialog.wh-dialog[closedby="any"]').forEach(function(dlg) {
            var pressedOutside = false;
            dlg.addEventListener('pointerdown', function(e) {
                pressedOutside = outsideDialog(e, dlg);
            });
            dlg.addEventListener('pointerup', function(e) {
                if (pressedOutside && outsideDialog(e, dlg)) {
                    dlg.close();
                }
            });
        });
    }

    [createDialog, deleteDialog].forEach(function(dlg) {
        if (!dlg) return;
        dlg.addEventListener('close', function() {
            dlg.returnValue = '';
        });
    });

    document.querySelectorAll('[data-dialog-close]').forEach(function(btn) {
        btn.addEventListener('click', function() {
            var id = btn.getAttribute('data-dialog-close');
            var dlg = id ? document.getElementById(id) : null;
            if (dlg && dlg.open) {
                dlg.close();
            }
        });
    });

    function showFeedback(message, isError) {
        if (!feedbackEl) return;
        clearTimeout(feedbackTimer);
        feedbackEl.textContent = message;
        feedbackEl.classList.remove('d-none', 'is-success', 'is-error');
        feedbackEl.classList.add(isError ? 'is-error' : 'is-success');
        feedbackTimer = setTimeout(function() {
            feedbackEl.classList.add('d-none');
        }, 6000);
    }

    function syncDigestVisibility(selectEl) {
        var form = selectEl.closest('form');
        if (!form) return;
        var freq = selectEl.value;
        var hourWrap = form.querySelector('.wh-digest-hour-wrap');
        var dayWrap = form.querySelector('.wh-digest-day-wrap');
        var skipWrap = form.querySelector('.wh-digest-skip-wrap');
        if (hourWrap) hourWrap.classList.toggle('d-none', freq === 'none');
        if (skipWrap) skipWrap.classList.toggle('d-none', freq === 'none');
        if (dayWrap) dayWrap.classList.toggle('d-none', freq !== 'weekly');
    }

    document.querySelectorAll('.wh-digest-freq').forEach(function(selectEl) {
        syncDigestVisibility(selectEl);
        selectEl.addEventListener('change', function() {
            syncDigestVisibility(selectEl);
        });
    });

    function syncEnabledState(switchEl) {
        var form = switchEl.closest('.wh-update-form');
        if (!form) return;
        var enabled = !!switchEl.checked;
        var fieldset = form.querySelector('.wh-triggers-fieldset');
        if (fieldset) {
            fieldset.disabled = !enabled;
        }
        var bar = form.querySelector('.wh-enable-bar');
        if (bar) {
            bar.classList.toggle('is-paused', !enabled);
        }
        var desc = form.querySelector('.wh-enable-desc');
        if (desc) {
            desc.textContent = enabled
                ? 'Active \u2014 matching events will be sent to Discord'
                : 'Paused \u2014 notifications are disabled until turned back on';
        }
    }

    var nativeSwitchSupported = 'switch' in HTMLInputElement.prototype;
    document.querySelectorAll('.wh-switch-input').forEach(function(switchEl) {
        if (!nativeSwitchSupported) {
            switchEl.setAttribute('role', 'switch');
        }
        syncEnabledState(switchEl);
        switchEl.addEventListener('change', function() {
            syncEnabledState(switchEl);
        });
    });

    function setCreateError(message) {
        if (!createErrorEl) return;
        if (message) {
            createErrorEl.textContent = message;
            createErrorEl.classList.remove('d-none');
        } else {
            createErrorEl.textContent = '';
            createErrorEl.classList.add('d-none');
        }
    }

    function openCreateModal() {
        if (!createDialog || !createForm) return;
        createForm.reset();
        setCreateError('');
        var freqSelect = createForm.querySelector('.wh-digest-freq');
        if (freqSelect) syncDigestVisibility(freqSelect);
        createDialog.showModal();
    }

    var openCreateBtn = document.getElementById('wh-open-create-btn');
    if (openCreateBtn) openCreateBtn.addEventListener('click', openCreateModal);

    var emptyCreateBtn = document.getElementById('wh-empty-create-btn');
    if (emptyCreateBtn) emptyCreateBtn.addEventListener('click', openCreateModal);

    function collectPayload(form, includeUrlAlways) {
        var getChecked = function(name) {
            var el = form.querySelector('input[name="' + name + '"]');
            return !!(el && el.checked);
        };
        var getValue = function(name, fallback) {
            var el = form.querySelector('[name="' + name + '"]');
            return el ? el.value : fallback;
        };

        var payload = {
            label: getValue('label', '').trim(),
            enabled: form.querySelector('input[name="enabled"]') ? getChecked('enabled') : true,
            on_suggestion_submitted: getChecked('on_suggestion_submitted'),
            on_suggestion_approved: getChecked('on_suggestion_approved'),
            on_suggestion_denied: getChecked('on_suggestion_denied'),
            on_direct_push: getChecked('on_direct_push'),
            on_changelog_published: getChecked('on_changelog_published'),
            on_maintainer_changed: getChecked('on_maintainer_changed'),
            on_subscriber_milestone: getChecked('on_subscriber_milestone'),
            digest_frequency: getValue('digest_frequency', 'none'),
            digest_hour_utc: parseInt(getValue('digest_hour_utc', '9'), 10),
            digest_day_of_week: parseInt(getValue('digest_day_of_week', '1'), 10),
            skip_empty_digest: getChecked('skip_empty_digest')
        };

        var urlVal = getValue('webhook_url', '').trim();
        if (includeUrlAlways || urlVal.length > 0) {
            payload.webhook_url = urlVal;
        }
        return payload;
    }

    function updateSummaryRow(itemEl, updated) {
        if (!itemEl || !updated) return;

        var titleEl = itemEl.querySelector('.wh-item-name');
        if (titleEl) {
            titleEl.textContent = updated.label || ('Discord Webhook #' + updated.id);
        }

        var codeEl = itemEl.querySelector('.wh-url-code');
        if (codeEl && updated.url_preview) {
            codeEl.textContent = updated.url_preview;
        }

        var statusEl = itemEl.querySelector('.wh-status');
        if (statusEl) {
            statusEl.className = 'wh-status ' + (
                updated.enabled
                    ? 'wh-status--active'
                    : (updated.disabled_reason ? 'wh-status--disabled' : 'wh-status--paused')
            );
            var textNode = Array.prototype.find.call(statusEl.childNodes, function(n) {
                return n.nodeType === Node.TEXT_NODE;
            });
            var statusLabel = updated.enabled
                ? 'Active'
                : (updated.disabled_reason ? 'Auto-disabled' : 'Paused');
            if (textNode) {
                textNode.textContent = statusLabel;
            } else {
                statusEl.appendChild(document.createTextNode(statusLabel));
            }
        }

        var triggersEl = itemEl.querySelector('.wh-summary-triggers');
        if (triggersEl && typeof updated.active_event_count === 'number') {
            var count = updated.active_event_count;
            var summaryText = count + ' event' + (count === 1 ? '' : 's');
            if (updated.digest_frequency && updated.digest_frequency !== 'none') {
                var cap = updated.digest_frequency.charAt(0).toUpperCase() + updated.digest_frequency.slice(1);
                summaryText += ' \u00B7 ' + cap + ' digest';
            }
            triggersEl.textContent = summaryText;
        }

        var metaEl = itemEl.querySelector('.wh-summary-meta');
        if (metaEl && updated.last_delivered_at) {
            var deliveredWrap = metaEl.querySelector('.wh-last-delivered-wrap');
            if (!deliveredWrap) {
                deliveredWrap = document.createElement('span');
                deliveredWrap.className = 'wh-last-delivered-wrap';
                var sep = document.createElement('span');
                sep.setAttribute('aria-hidden', 'true');
                sep.textContent = '\u00B7';
                var timeSpan = document.createElement('span');
                timeSpan.className = 'di-num wh-last-delivered';
                deliveredWrap.appendChild(sep);
                deliveredWrap.appendChild(document.createTextNode(' Last delivered '));
                deliveredWrap.appendChild(timeSpan);
                metaEl.appendChild(deliveredWrap);
            }
            var deliveredEl = deliveredWrap.querySelector('.wh-last-delivered');
            if (deliveredEl) deliveredEl.textContent = updated.last_delivered_at;
        }

        var bodyEl = itemEl.querySelector('.wh-item-body');
        if (bodyEl) {
            var existingReason = bodyEl.querySelector('.wh-alert-reason');
            var existingErr = bodyEl.querySelector('.wh-alert-error');
            if (updated.disabled_reason) {
                if (existingErr) existingErr.remove();
                if (!existingReason) {
                    existingReason = document.createElement('div');
                    existingReason.className = 'wh-notice wh-notice-danger wh-alert-reason';
                    existingReason.setAttribute('role', 'alert');
                    bodyEl.insertBefore(existingReason, bodyEl.firstChild);
                }
                existingReason.textContent = updated.disabled_reason;
            } else if (updated.last_error) {
                if (existingReason) existingReason.remove();
                if (!existingErr) {
                    existingErr = document.createElement('div');
                    existingErr.className = 'wh-notice wh-notice-warn wh-alert-error';
                    existingErr.setAttribute('role', 'alert');
                    bodyEl.insertBefore(existingErr, bodyEl.firstChild);
                }
                existingErr.textContent = 'Last delivery error: ' + updated.last_error;
            } else {
                if (existingReason) existingReason.remove();
                if (existingErr) existingErr.remove();
            }
        }

        var switchEl = itemEl.querySelector('.wh-switch-input');
        if (switchEl) {
            switchEl.checked = !!updated.enabled;
            syncEnabledState(switchEl);
        }
    }

    function updateCountAndBanner() {
        var items = document.querySelectorAll('#webhooks-list .wh-item');
        var count = items.length;

        var countEl = document.getElementById('webhook-count');
        if (countEl) countEl.textContent = String(count);

        var listEl = document.getElementById('webhooks-list');
        if (listEl) listEl.classList.toggle('d-none', count === 0);

        var emptyEl = document.getElementById('webhooks-empty-state');
        if (emptyEl) emptyEl.classList.toggle('d-none', count > 0);

        if (openCreateBtn) openCreateBtn.classList.toggle('d-none', count >= maxWebhooks);
        var limitNote = document.getElementById('wh-limit-note');
        if (limitNote) limitNote.classList.toggle('d-none', count < maxWebhooks);
    }

    if (createForm) {
        createForm.addEventListener('submit', function(event) {
            event.preventDefault();
            setCreateError('');
            if (!createForm.checkValidity()) {
                var firstInvalidCreate = createForm.querySelector(':invalid');
                if (firstInvalidCreate) firstInvalidCreate.focus();
                return;
            }

            var submitBtn = createForm.querySelector('button[type="submit"]');
            if (submitBtn) {
                if (submitBtn.disabled) return;
                submitBtn.disabled = true;
            }

            var payload = collectPayload(createForm, true);
            window.ApiService.apiCall('/api/decks/' + encodeURIComponent(deckHash) + '/webhooks', 'POST', payload)
                .then(function() {
                    if (createDialog && createDialog.open) createDialog.close();
                    window.location.reload();
                })
                .catch(function(err) {
                    var msg = (err && err.message) || 'Failed to add webhook.';
                    if (createDialog && createDialog.open) {
                        setCreateError(msg);
                        var urlField = document.getElementById('new-wh-url');
                        if (urlField) urlField.focus();
                    } else {
                        showFeedback(msg, true);
                    }
                })
                .finally(function() {
                    if (submitBtn) submitBtn.disabled = false;
                });
        });
    }

    document.querySelectorAll('.wh-update-form').forEach(function(form) {
        form.addEventListener('submit', function(event) {
            event.preventDefault();
            if (!form.checkValidity()) {
                var firstInvalidUpdate = form.querySelector(':invalid');
                if (firstInvalidUpdate) firstInvalidUpdate.focus();
                return;
            }

            var webhookId = form.dataset.webhookId;
            var submitBtn = form.querySelector('button[type="submit"]');
            if (submitBtn) {
                if (submitBtn.disabled) return;
                submitBtn.disabled = true;
            }

            var payload = collectPayload(form, false);
            window.ApiService.apiCall(
                '/api/decks/' + encodeURIComponent(deckHash) + '/webhooks/' + encodeURIComponent(webhookId) + '/update',
                'POST',
                payload
            )
                .then(function(updated) {
                    var itemEl = form.closest('.wh-item');
                    updateSummaryRow(itemEl, updated);
                    var urlInput = form.querySelector('input[name="webhook_url"]');
                    if (urlInput) {
                        urlInput.value = '';
                        urlInput.classList.remove('user-invalid-fallback');
                        syncAria(urlInput, false);
                    }
                    showFeedback('Webhook changes saved.', false);
                })
                .catch(function(err) {
                    showFeedback((err && err.message) || 'Failed to save changes.', true);
                })
                .finally(function() {
                    if (submitBtn) submitBtn.disabled = false;
                });
        });
    });

    document.querySelectorAll('.wh-test-btn').forEach(function(btn) {
        btn.addEventListener('click', function() {
            var webhookId = btn.dataset.webhookId;
            if (btn.disabled) return;
            btn.disabled = true;

            window.ApiService.apiCall(
                '/api/decks/' + encodeURIComponent(deckHash) + '/webhooks/' + encodeURIComponent(webhookId) + '/test',
                'POST'
            )
                .then(function(res) {
                    if (res && res.webhook) {
                        updateSummaryRow(btn.closest('.wh-item'), res.webhook);
                    }
                    showFeedback((res && res.message) || 'Test notification sent to Discord.', false);
                })
                .catch(function(err) {
                    showFeedback((err && err.message) || 'Test notification failed.', true);
                })
                .finally(function() {
                    btn.disabled = false;
                });
        });
    });

    document.querySelectorAll('.wh-delete-btn').forEach(function(btn) {
        btn.addEventListener('click', function() {
            pendingDeleteId = btn.dataset.webhookId;
            pendingDeleteCard = btn.closest('.wh-item');
            var targetName = pendingDeleteCard
                ? (pendingDeleteCard.querySelector('.wh-item-name') || {}).textContent
                : null;
            var targetEl = document.getElementById('wh-delete-target');
            if (targetEl) {
                targetEl.textContent = targetName || ('Webhook #' + pendingDeleteId);
            }
            if (deleteDialog) deleteDialog.showModal();
        });
    });

    var confirmDeleteBtn = document.getElementById('wh-confirm-delete-btn');
    if (confirmDeleteBtn) {
        confirmDeleteBtn.addEventListener('click', function() {
            if (!pendingDeleteId || confirmDeleteBtn.disabled) return;
            confirmDeleteBtn.disabled = true;
            var idToDelete = pendingDeleteId;
            var itemToRemove = pendingDeleteCard;

            window.ApiService.apiCall(
                '/api/decks/' + encodeURIComponent(deckHash) + '/webhooks/' + encodeURIComponent(idToDelete) + '/delete',
                'POST'
            )
                .then(function() {
                    if (deleteDialog && deleteDialog.open) deleteDialog.close();
                    if (itemToRemove) itemToRemove.remove();
                    updateCountAndBanner();
                    showFeedback('Webhook deleted.', false);
                })
                .catch(function(err) {
                    if (deleteDialog && deleteDialog.open) deleteDialog.close();
                    showFeedback((err && err.message) || 'Failed to delete webhook.', true);
                })
                .finally(function() {
                    pendingDeleteId = null;
                    pendingDeleteCard = null;
                    confirmDeleteBtn.disabled = false;
                });
        });
    }
});
