/**
 * subscription_policy.js - Subscription field policy management
 * Reads subscriber_hash and base_hash from data attributes on #policy-root
 */
document.addEventListener('DOMContentLoaded', function() {
    var policyRoot = document.getElementById('policy-root');
    if (!policyRoot) return;
    
    var subscriberHash = policyRoot.dataset.subscriberHash || '';
    var baseHash = policyRoot.dataset.baseHash || '';

    var state = {
        notetypes: [], // [{id, name, fields: [{position, name, protected}], selected: Set<int> | null }]
        existingPolicies: new Map(), // notetype_id -> subscribed_fields (array|null)
    };

    function render() {
        var root = document.getElementById('policy-root');
        root.innerHTML = '';
        state.notetypes.forEach(function(nt) {
            var card = document.createElement('div');
            card.className = 'card mb-3';
            var body = document.createElement('div');
            body.className = 'card-body';
            var h5 = document.createElement('h5');
            h5.className = 'card-title';
            h5.textContent = nt.name + ' (ID ' + nt.id + ')';
            body.appendChild(h5);

            var selectAllDiv = document.createElement('div');
            selectAllDiv.className = 'form-check mb-2 subscribe-all';
            var selectAll = document.createElement('input');
            selectAll.type = 'checkbox';
            selectAll.className = 'form-check-input';
            var selectAllId = 'subscribe_all_' + nt.id;
            selectAll.id = selectAllId;
            var hasProtected = nt.fields.some(function(f) { return f.protected; });
            selectAll.disabled = hasProtected;
            selectAll.checked = !hasProtected && nt.selected === null;
            selectAll.addEventListener('change', function() {
                if (selectAll.disabled) return;
                if (selectAll.checked) {
                    nt.selected = null;
                } else {
                    nt.selected = new Set(nt.fields.filter(function(f) { return !f.protected; }).map(function(f) { return f.position; }));
                }
                render();
            });
            var selectAllLbl = document.createElement('label');
            selectAllLbl.className = 'form-check-label ml-2';
            selectAllLbl.htmlFor = selectAllId;
            selectAllLbl.textContent = 'Subscribe all' + (selectAll.disabled ? ' (disabled: protected fields present)' : '');
            selectAllDiv.appendChild(selectAll);
            selectAllDiv.appendChild(selectAllLbl);
            body.appendChild(selectAllDiv);

            var list = document.createElement('div');
            nt.fields.forEach(function(f) {
                var item = document.createElement('div');
                item.className = 'form-check';
                var cb = document.createElement('input');
                cb.type = 'checkbox';
                cb.className = 'form-check-input';
                cb.disabled = f.protected || nt.selected === null;
                var checked = nt.selected === null ? true : nt.selected.has(f.position);
                cb.checked = checked;
                cb.addEventListener('change', function() {
                    if (nt.selected === null) return;
                    if (cb.checked) nt.selected.add(f.position); else nt.selected.delete(f.position);
                });
                var label = document.createElement('label');
                label.className = 'form-check-label';
                label.textContent = f.name + (f.protected ? ' (protected)' : '');
                item.appendChild(cb);
                item.appendChild(label);
                list.appendChild(item);
            });
            body.appendChild(list);

            card.appendChild(body);
            root.appendChild(card);
        });
    }

    async function loadPolicies() {
        var resp = await fetch('/api/subscription-field-policy?subscriber_deck_hash=' + encodeURIComponent(subscriberHash) + '&base_deck_hash=' + encodeURIComponent(baseHash));
        if (!resp.ok) return;
        var data = await resp.json();
        state.existingPolicies = new Map((data.policies || []).map(function(p) { return [p.notetype_id, p.subscribed_fields]; }));
    }

    async function loadNotetypes() {
        var metaElement = document.getElementById('notetype_meta');
        if (!metaElement) return;
        var meta = JSON.parse(metaElement.textContent);
        state.notetypes = meta.map(function(nt) {
            return {
                id: nt.id,
                name: nt.name,
                fields: nt.fields,
                selected: undefined,
            };
        });
        // apply existing policies
        state.notetypes.forEach(function(nt) {
            var sf = state.existingPolicies.get(nt.id);
            var hasProtected = nt.fields.some(function(f) { return f.protected; });
            if (sf === undefined) {
                nt.selected = new Set();
            } else if (sf === null) {
                nt.selected = hasProtected ? new Set(nt.fields.filter(function(f) { return !f.protected; }).map(function(f) { return f.position; })) : null;
            } else {
                var allowed = new Set(nt.fields.filter(function(f) { return !f.protected; }).map(function(f) { return f.position; }));
                nt.selected = new Set(sf.filter(function(p) { return allowed.has(p); }));
            }
        });
        render();
    }

    async function savePolicy() {
        var policies = state.notetypes.map(function(nt) {
            return {
                notetype_id: nt.id,
                subscribed_fields: nt.selected === null ? null : Array.from(nt.selected),
            };
        });
        var resp = await fetch('/api/subscription-field-policy', {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify({
                subscriber_deck_hash: subscriberHash,
                base_deck_hash: baseHash,
                policies: policies,
            }),
        });
        if (resp.ok) {
            alert('Policy saved successfully!');
        } else {
            alert('Failed to save policy');
        }
    }

    // Initialize
    (async function() {
        await loadPolicies();
        await loadNotetypes();
    })();

    // Save button handler
    var saveBtn = document.getElementById('saveBtn');
    if (saveBtn) {
        saveBtn.addEventListener('click', savePolicy);
    }
});
