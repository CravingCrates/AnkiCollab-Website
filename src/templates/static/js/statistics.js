/* CSP-compliant deck statistics behavior: Chart.js reads data-* payloads and adapts to
 * light/dark color-scheme changes without inline scripts or localStorage. */
(function ($) {
    'use strict';

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
                accent: '#4aa8d8',
                accentFill: 'rgba(74, 168, 216, 0.16)',
                success: '#4ade80',
                warning: '#fbbf24',
                danger: '#f87171',
                secondary: '#94a3b8',
                text: '#cbd5e1',
                grid: 'rgba(255, 255, 255, 0.08)',
                border: 'rgba(255, 255, 255, 0.16)',
                tooltipBg: '#242428',
                tooltipTitle: '#f5f5f7',
                tooltipBody: '#cbd5e1',
                tooltipBorder: '#38383d'
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

    $(function () {
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

        var activeFilter = 'all';
        var notesEl = $('#notesOverview');
        var notesTable = null;

        if (notesEl.length) {
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

        var colorSchemeQuery = window.matchMedia
            ? window.matchMedia('(prefers-color-scheme: dark)')
            : null;
        var isDark = Boolean(colorSchemeQuery && colorSchemeQuery.matches);
        var theme = getTheme(isDark);
        var themeUpdaters = [];

        var distCanvas = document.getElementById('retentionDistChart');
        if (distCanvas) {
            var allSeries = parseJsonAttr(distCanvas, 'all');
            var confidentSeries = parseJsonAttr(distCanvas, 'confident');

            var distChart = new Chart(distCanvas.getContext('2d'), {
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

                distChart.data.datasets[0].data = mode === 'confident' ? confidentSeries : allSeries;
                distChart.update('none');
            });

            themeUpdaters.push(function (next) {
                distChart.data.datasets[0].backgroundColor = distBarColors(next);
                distChart.options.plugins.tooltip.backgroundColor = next.tooltipBg;
                distChart.options.plugins.tooltip.titleColor = next.tooltipTitle;
                distChart.options.plugins.tooltip.bodyColor = next.tooltipBody;
                distChart.options.plugins.tooltip.borderColor = next.tooltipBorder;
                distChart.options.scales.y.border.color = next.border;
                distChart.options.scales.y.grid.color = next.grid;
                distChart.options.scales.y.ticks.color = next.text;
                distChart.options.scales.x.border.color = next.border;
                distChart.options.scales.x.ticks.color = next.text;
                distChart.update('none');
            });
        }

        var trendCanvas = document.getElementById('retentionTrendChart');
        if (trendCanvas) {
            var trendChart = new Chart(trendCanvas.getContext('2d'), {
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
                trendChart.data.datasets[0].borderColor = next.accent;
                trendChart.data.datasets[0].backgroundColor = next.accentFill;
                trendChart.data.datasets[1].borderColor = next.secondary;
                trendChart.options.plugins.legend.labels.color = next.text;
                trendChart.options.plugins.tooltip.backgroundColor = next.tooltipBg;
                trendChart.options.plugins.tooltip.titleColor = next.tooltipTitle;
                trendChart.options.plugins.tooltip.bodyColor = next.tooltipBody;
                trendChart.options.plugins.tooltip.borderColor = next.tooltipBorder;
                trendChart.options.scales.y.border.color = next.border;
                trendChart.options.scales.y.grid.color = next.grid;
                trendChart.options.scales.y.ticks.color = next.text;
                trendChart.options.scales.yLapses.ticks.color = next.secondary;
                trendChart.options.scales.x.border.color = next.border;
                trendChart.options.scales.x.ticks.color = next.text;
                trendChart.update('none');
            });
        }

        if (colorSchemeQuery && themeUpdaters.length) {
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
    });
}(jQuery));
