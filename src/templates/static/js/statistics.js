/* CSP-compliant deck statistics behavior: supports both instant SSR and progressive
 * deferred partial hydration, Chart.js theming, DataTables, and zero localStorage. */
(function ($) {
    'use strict';

    var activeFilter = 'all';
    var searchFilterRegistered = false;
    var schemeListenerRegistered = false;
    var themeUpdaters = [];
    var distChartInstance = null;
    var trendChartInstance = null;

    function parseJsonAttr(el, attrName) {
        var raw = el.getAttribute('data-' + attrName);
        if (!raw) {
            return [];
        }
        try {
            return JSON.parse(raw) || [];
        } catch (_err) {
            return [];
        }
    }

    function getTheme(isDark) {
        if (isDark) {
            return {
                accent: '#5cb8e6',
                accentFill: 'rgba(92, 184, 230, 0.18)',
                success: '#6ee7b7',
                warning: '#fde68a',
                danger: '#fca5a5',
                secondary: '#a1aec0',
                text: '#d5dbe3',
                grid: 'rgba(255, 255, 255, 0.10)',
                border: 'rgba(255, 255, 255, 0.20)',
                tooltipBg: '#28282f',
                tooltipTitle: '#f5f5f7',
                tooltipBody: '#d5dbe3',
                tooltipBorder: '#464652'
            };
        }
        return {
            accent: '#006699',
            accentFill: 'rgba(0, 102, 153, 0.12)',
            success: '#087443',
            warning: '#b45309',
            danger: '#b42318',
            secondary: '#64748b',
            text: '#4b5563',
            grid: 'rgba(15, 23, 42, 0.08)',
            border: 'rgba(15, 23, 42, 0.14)',
            tooltipBg: '#1d1d1f',
            tooltipTitle: '#ffffff',
            tooltipBody: '#f5f5f7',
            tooltipBorder: '#1d1d1f'
        };
    }

    function distBarColors(theme) {
        // 7 bands: <60, 60-70, 70-80, 80-85, 85-90, 90-95, 95-100
        return [
            theme.danger,
            theme.danger,
            theme.warning,
            theme.accent,
            theme.success,
            theme.success,
            theme.accent
        ];
    }

    function announceStatus(message) {
        var region = document.getElementById('a11y-status');
        if (region) {
            region.textContent = message;
        }
    }

    function announceAlert(message) {
        var region = document.getElementById('a11y-alert') || document.getElementById('a11y-status');
        if (region) {
            region.textContent = message;
        }
    }

    function registerNotesSearchFilter() {
        if (searchFilterRegistered || !$.fn.dataTable || !$.fn.dataTable.ext) {
            return;
        }
        searchFilterRegistered = true;

        $.fn.dataTable.ext.search.push(function (settings, _data, dataIndex) {
            if (!settings || !settings.nTable || settings.nTable.id !== 'notesOverview') {
                return true;
            }
            if (activeFilter === 'all') {
                return true;
            }
            var rowNode = settings.aoData[dataIndex] && settings.aoData[dataIndex].nTr;
            if (!rowNode) {
                return true;
            }
            var confident = rowNode.getAttribute('data-confident') === '1';
            var leech = rowNode.getAttribute('data-leech') === '1';
            var revised = rowNode.getAttribute('data-revised') === '1';
            var pending = rowNode.getAttribute('data-pending') === '1';

            if (activeFilter === 'attention') {
                return confident && !pending && !revised;
            }
            if (activeFilter === 'confident') {
                return confident;
            }
            if (activeFilter === 'leech') {
                return leech;
            }
            if (activeFilter === 'revised') {
                return revised;
            }
            return true;
        });
    }

    function registerSchemeListener(colorSchemeQuery) {
        if (schemeListenerRegistered || !colorSchemeQuery) {
            return;
        }
        schemeListenerRegistered = true;

        var handleSchemeChange = function (event) {
            var next = getTheme(event.matches);
            themeUpdaters.forEach(function (fn) {
                fn(next);
            });
        };
        if (typeof colorSchemeQuery.addEventListener === 'function') {
            colorSchemeQuery.addEventListener('change', handleSchemeChange);
        } else if (typeof colorSchemeQuery.addListener === 'function') {
            colorSchemeQuery.addListener(handleSchemeChange);
        }
    }

    function destroyCharts() {
        if (distChartInstance) {
            distChartInstance.destroy();
            distChartInstance = null;
        }
        if (trendChartInstance) {
            trendChartInstance.destroy();
            trendChartInstance = null;
        }
        themeUpdaters = [];
    }

    function initStatisticsContent() {
        var deckTable = $('#deckOverview');
        if (deckTable.length) {
            deckTable.DataTable({
                destroy: true,
                pageLength: 10,
                order: [],
                stripeClasses: []
            });
        }

        var tagsTable = $('#tagsOverview');
        if (tagsTable.length) {
            tagsTable.DataTable({
                destroy: true,
                pageLength: 10,
                order: [],
                stripeClasses: []
            });
        }

        activeFilter = 'all';
        var notesEl = $('#notesOverview');
        var notesTable = null;

        if (notesEl.length) {
            registerNotesSearchFilter();

            notesTable = notesEl.DataTable({
                destroy: true,
                pageLength: 25,
                order: [],
                stripeClasses: []
            });

            $('[data-notes-filter]').on('click', function () {
                var btn = $(this);
                var nextFilter = btn.attr('data-notes-filter') || 'all';
                activeFilter = nextFilter;

                $('[data-notes-filter]')
                    .removeClass('is-active')
                    .attr('aria-pressed', 'false');
                btn.addClass('is-active').attr('aria-pressed', 'true');

                notesTable.draw();
                var count = notesTable.rows({ filter: 'applied' }).count();
                announceStatus(count + (count === 1 ? ' note shown' : ' notes shown'));
            });
        }

        if (typeof Chart === 'undefined') {
            return;
        }

        destroyCharts();

        var colorSchemeQuery = window.matchMedia
            ? window.matchMedia('(prefers-color-scheme: dark)')
            : null;
        var isDark = Boolean(colorSchemeQuery && colorSchemeQuery.matches);
        var theme = getTheme(isDark);

        var distCanvas = document.getElementById('retentionDistChart');
        if (distCanvas) {
            var allSeries = parseJsonAttr(distCanvas, 'all');
            var confidentSeries = parseJsonAttr(distCanvas, 'confident');

            distChartInstance = new Chart(distCanvas.getContext('2d'), {
                type: 'bar',
                data: {
                    labels: parseJsonAttr(distCanvas, 'labels'),
                    datasets: [
                        {
                            label: 'Notes',
                            data: allSeries,
                            backgroundColor: distBarColors(theme),
                            borderRadius: 6,
                            borderSkipped: false,
                            maxBarThickness: 42
                        }
                    ]
                },
                options: {
                    responsive: true,
                    maintainAspectRatio: false,
                    animation: false,
                    plugins: {
                        legend: { display: false },
                        tooltip: {
                            backgroundColor: theme.tooltipBg,
                            titleColor: theme.tooltipTitle,
                            bodyColor: theme.tooltipBody,
                            borderColor: theme.tooltipBorder,
                            borderWidth: 1
                        }
                    },
                    scales: {
                        y: {
                            beginAtZero: true,
                            border: { color: theme.border },
                            grid: { color: theme.grid },
                            ticks: { precision: 0, color: theme.text }
                        },
                        x: {
                            border: { color: theme.border },
                            grid: { display: false },
                            ticks: { color: theme.text }
                        }
                    }
                }
            });

            $('[data-dist-mode]').on('click', function () {
                var btn = $(this);
                var mode = btn.attr('data-dist-mode');
                $('[data-dist-mode]')
                    .removeClass('is-active')
                    .attr('aria-pressed', 'false');
                btn.addClass('is-active').attr('aria-pressed', 'true');

                if (distChartInstance) {
                    distChartInstance.data.datasets[0].data = mode === 'confident' ? confidentSeries : allSeries;
                    distChartInstance.update('none');
                }
            });

            themeUpdaters.push(function (next) {
                if (!distChartInstance) {
                    return;
                }
                distChartInstance.data.datasets[0].backgroundColor = distBarColors(next);
                distChartInstance.options.plugins.tooltip.backgroundColor = next.tooltipBg;
                distChartInstance.options.plugins.tooltip.titleColor = next.tooltipTitle;
                distChartInstance.options.plugins.tooltip.bodyColor = next.tooltipBody;
                distChartInstance.options.plugins.tooltip.borderColor = next.tooltipBorder;
                distChartInstance.options.scales.y.border.color = next.border;
                distChartInstance.options.scales.y.grid.color = next.grid;
                distChartInstance.options.scales.y.ticks.color = next.text;
                distChartInstance.options.scales.x.border.color = next.border;
                distChartInstance.options.scales.x.ticks.color = next.text;
                distChartInstance.update('none');
            });
        }

        var trendCanvas = document.getElementById('retentionTrendChart');
        if (trendCanvas) {
            trendChartInstance = new Chart(trendCanvas.getContext('2d'), {
                type: 'line',
                data: {
                    labels: parseJsonAttr(trendCanvas, 'labels'),
                    datasets: [
                        {
                            label: 'Average retention (%)',
                            data: parseJsonAttr(trendCanvas, 'retention'),
                            borderColor: theme.accent,
                            backgroundColor: theme.accentFill,
                            fill: true,
                            tension: 0.25,
                            pointRadius: 2,
                            borderWidth: 2,
                            yAxisID: 'y'
                        },
                        {
                            label: 'Average lapses',
                            data: parseJsonAttr(trendCanvas, 'lapses'),
                            borderColor: theme.secondary,
                            fill: false,
                            tension: 0.25,
                            pointRadius: 0,
                            borderWidth: 1.5,
                            borderDash: [4, 4],
                            yAxisID: 'yLapses'
                        }
                    ]
                },
                options: {
                    responsive: true,
                    maintainAspectRatio: false,
                    animation: false,
                    interaction: {
                        mode: 'index',
                        intersect: false
                    },
                    plugins: {
                        legend: {
                            position: 'bottom',
                            labels: {
                                boxWidth: 12,
                                usePointStyle: true,
                                color: theme.text,
                                font: { size: 11 }
                            }
                        },
                        tooltip: {
                            backgroundColor: theme.tooltipBg,
                            titleColor: theme.tooltipTitle,
                            bodyColor: theme.tooltipBody,
                            borderColor: theme.tooltipBorder,
                            borderWidth: 1
                        }
                    },
                    scales: {
                        y: {
                            position: 'left',
                            suggestedMin: 50,
                            suggestedMax: 100,
                            border: { color: theme.border },
                            grid: { color: theme.grid },
                            ticks: { color: theme.text }
                        },
                        yLapses: {
                            position: 'right',
                            beginAtZero: true,
                            border: { display: false },
                            grid: { drawOnChartArea: false },
                            ticks: { color: theme.secondary }
                        },
                        x: {
                            border: { color: theme.border },
                            grid: { display: false },
                            ticks: {
                                color: theme.text,
                                autoSkip: true,
                                maxTicksLimit: 7,
                                maxRotation: 0,
                                minRotation: 0
                            }
                        }
                    }
                }
            });

            themeUpdaters.push(function (next) {
                if (!trendChartInstance) {
                    return;
                }
                trendChartInstance.data.datasets[0].borderColor = next.accent;
                trendChartInstance.data.datasets[0].backgroundColor = next.accentFill;
                trendChartInstance.data.datasets[1].borderColor = next.secondary;
                trendChartInstance.options.plugins.legend.labels.color = next.text;
                trendChartInstance.options.plugins.tooltip.backgroundColor = next.tooltipBg;
                trendChartInstance.options.plugins.tooltip.titleColor = next.tooltipTitle;
                trendChartInstance.options.plugins.tooltip.bodyColor = next.tooltipBody;
                trendChartInstance.options.plugins.tooltip.borderColor = next.tooltipBorder;
                trendChartInstance.options.scales.y.border.color = next.border;
                trendChartInstance.options.scales.y.grid.color = next.grid;
                trendChartInstance.options.scales.y.ticks.color = next.text;
                trendChartInstance.options.scales.yLapses.ticks.color = next.secondary;
                trendChartInstance.options.scales.x.border.color = next.border;
                trendChartInstance.options.scales.x.ticks.color = next.text;
                trendChartInstance.update('none');
            });
        }

        registerSchemeListener(colorSchemeQuery);
    }

    function renderDeferredError(mount, syncUrl) {
        while (mount.firstChild) {
            mount.removeChild(mount.firstChild);
        }

        var card = document.createElement('section');
        card.className = 'di-card di-deferred-error';

        var heading = document.createElement('h2');
        heading.className = 'di-card-title mb-2';
        heading.textContent = 'Unable to load detailed statistics';
        card.appendChild(heading);

        var desc = document.createElement('p');
        desc.className = 'di-card-subtitle mb-0';
        desc.textContent = 'The request took too long or was interrupted. Retry loading or open the full synchronous view.';
        card.appendChild(desc);

        var actions = document.createElement('div');
        actions.className = 'di-deferred-actions';

        var retryBtn = document.createElement('button');
        retryBtn.type = 'button';
        retryBtn.className = 'btn btn-primary btn-sm';
        retryBtn.textContent = 'Try again';
        retryBtn.addEventListener('click', function () {
            retryBtn.disabled = true;
            retryBtn.textContent = 'Loading…';
            loadDeferredStats(mount);
        });
        actions.appendChild(retryBtn);

        if (syncUrl && syncUrl.charAt(0) === '/') {
            var syncLink = document.createElement('a');
            syncLink.className = 'btn btn-outline-secondary btn-sm';
            syncLink.href = syncUrl;
            syncLink.textContent = 'Load synchronously';
            actions.appendChild(syncLink);
        }

        card.appendChild(actions);
        mount.appendChild(card);
    }

    function loadDeferredStats(mount) {
        var deferredUrl = mount.getAttribute('data-stats-deferred-url');
        var syncUrl = mount.getAttribute('data-stats-sync-url');
        if (!deferredUrl || deferredUrl.charAt(0) !== '/') {
            return;
        }

        mount.setAttribute('aria-busy', 'true');

        window.fetch(deferredUrl, {
            credentials: 'same-origin',
            headers: { 'X-Requested-With': 'XMLHttpRequest' }
        }).then(function (response) {
            if (response.redirected && response.url && response.url.indexOf('/login') !== -1) {
                window.location.assign(response.url);
                return null;
            }
            if (!response.ok) {
                throw new Error('HTTP ' + response.status);
            }
            return response.text();
        }).then(function (html) {
            if (html === null) {
                return;
            }
            mount.innerHTML = html;
            mount.removeAttribute('data-stats-deferred-url');
            mount.setAttribute('aria-busy', 'false');
            mount.classList.add('di-fade-in');
            initStatisticsContent();
            announceStatus('Deck statistics loaded.');
        }).catch(function () {
            mount.setAttribute('aria-busy', 'false');
            renderDeferredError(mount, syncUrl);
            announceAlert('Failed to load deck statistics.');
        });
    }

    $(function () {
        var mount = document.getElementById('statsDeferredMount');
        if (mount && mount.hasAttribute('data-stats-deferred-url')) {
            loadDeferredStats(mount);
        } else {
            initStatisticsContent();
        }
    });
}(jQuery));
