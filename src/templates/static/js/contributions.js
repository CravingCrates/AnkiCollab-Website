/* The site's CSP forbids inline <script>, so behaviour lives here and chart payloads arrive
 * on each <canvas> as data-* attributes. */
(function ($) {
    'use strict';

    function parseSeries(canvas, name) {
        var raw = canvas.getAttribute('data-' + name);
        if (!raw) {
            return [];
        }
        try {
            return JSON.parse(raw) || [];
        } catch (_err) {
            return [];
        }
    }

    function getChartTheme(isDark) {
        if (isDark) {
            return {
                primary: '#4aa8d8',
                primaryFill: 'rgba(74, 168, 216, 0.16)',
                subscribed: '#4ade80',
                subscribedBar: 'rgba(74, 222, 128, 0.45)',
                unsubscribed: '#94a3b8',
                unsubscribedBar: 'rgba(148, 163, 184, 0.45)',
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
            primary: '#006699',
            primaryFill: 'rgba(0, 102, 153, 0.12)',
            subscribed: '#087443',
            subscribedBar: 'rgba(8, 116, 67, 0.42)',
            unsubscribed: '#64748b',
            unsubscribedBar: 'rgba(100, 116, 139, 0.36)',
            text: '#4b5563',
            grid: 'rgba(15, 23, 42, 0.08)',
            border: 'rgba(15, 23, 42, 0.14)',
            tooltipBg: '#1d1d1f',
            tooltipTitle: '#ffffff',
            tooltipBody: '#f5f5f7',
            tooltipBorder: '#1d1d1f'
        };
    }

    $(function () {
        var timelineEl = $('#timelineOverview');
        var timelineTable = null;
        if (timelineEl.length) {
            timelineTable = timelineEl.DataTable({
                destroy: true,
                pageLength: 10,
                order: [[0, 'desc']],
                searching: false,
                stripeClasses: []
            });

            var disclosure = document.getElementById('timelineDisclosure');
            if (disclosure) {
                disclosure.addEventListener('toggle', function () {
                    if (disclosure.open && timelineTable) {
                        timelineTable.columns.adjust();
                    }
                });
            }
        }

        var contributorsEl = $('#contributorsOverview');
        if (contributorsEl.length) {
            contributorsEl.DataTable({
                destroy: true,
                pageLength: 25,
                order: [],
                stripeClasses: []
            });
        }

        var suggestionsEl = $('#suggestionOverview');
        if (suggestionsEl.length) {
            suggestionsEl.DataTable({
                destroy: true,
                pageLength: 25,
                order: [],
                stripeClasses: []
            });
        }

        if (typeof Chart === 'undefined') {
            return;
        }

        var colorSchemeQuery = window.matchMedia
            ? window.matchMedia('(prefers-color-scheme: dark)')
            : null;
        var theme = getChartTheme(Boolean(colorSchemeQuery && colorSchemeQuery.matches));
        var themeUpdaters = [];

        var subCanvas = document.getElementById('subscriberTimelineChart');
        if (subCanvas) {
            var tracked = subCanvas.getAttribute('data-tracked') === 'true';
            var view = subCanvas.getAttribute('data-view');
            var primary = parseSeries(subCanvas, 'series');
            var primaryLabel = 'Subscribers';
            if (tracked && view === '30') {
                primary = parseSeries(subCanvas, 'active-30d');
                primaryLabel = 'Active in the last 30 days';
            } else if (tracked && view === '90') {
                primary = parseSeries(subCanvas, 'active-90d');
                primaryLabel = 'Active in the last 90 days';
            }

            var subChart = new Chart(subCanvas.getContext('2d'), {
                data: {
                    labels: parseSeries(subCanvas, 'labels'),
                    datasets: [
                        {
                            type: 'line',
                            label: primaryLabel,
                            data: primary,
                            borderColor: theme.primary,
                            backgroundColor: theme.primaryFill,
                            fill: true,
                            tension: 0.2,
                            pointRadius: 0,
                            borderWidth: 2,
                            yAxisID: 'y',
                            order: 1
                        },
                        {
                            type: 'bar',
                            label: 'New subscribers',
                            data: parseSeries(subCanvas, 'new'),
                            backgroundColor: theme.subscribedBar,
                            borderColor: theme.subscribed,
                            borderWidth: 1,
                            borderRadius: 3,
                            maxBarThickness: 24,
                            yAxisID: 'yFlow',
                            order: 2
                        },
                        {
                            type: 'bar',
                            label: 'Unsubscribed',
                            data: parseSeries(subCanvas, 'unsubscribed'),
                            backgroundColor: theme.unsubscribedBar,
                            borderColor: theme.unsubscribed,
                            borderWidth: 1,
                            borderRadius: 3,
                            maxBarThickness: 24,
                            yAxisID: 'yFlow',
                            order: 3
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
                            beginAtZero: true,
                            border: { color: theme.border },
                            grid: { color: theme.grid },
                            ticks: { precision: 0, color: theme.text }
                        },
                        yFlow: {
                            position: 'right',
                            beginAtZero: true,
                            suggestedMax: 5,
                            border: { display: false },
                            grid: { drawOnChartArea: false },
                            ticks: { precision: 0, color: theme.unsubscribed }
                        },
                        x: {
                            border: { color: theme.border },
                            grid: { display: false },
                            ticks: {
                                color: theme.text,
                                autoSkip: true,
                                maxTicksLimit: 8,
                                maxRotation: 0,
                                minRotation: 0
                            }
                        }
                    }
                }
            });

            themeUpdaters.push(function (next) {
                subChart.data.datasets[0].borderColor = next.primary;
                subChart.data.datasets[0].backgroundColor = next.primaryFill;
                subChart.data.datasets[1].backgroundColor = next.subscribedBar;
                subChart.data.datasets[1].borderColor = next.subscribed;
                subChart.data.datasets[2].backgroundColor = next.unsubscribedBar;
                subChart.data.datasets[2].borderColor = next.unsubscribed;
                subChart.options.plugins.legend.labels.color = next.text;
                subChart.options.plugins.tooltip.backgroundColor = next.tooltipBg;
                subChart.options.plugins.tooltip.titleColor = next.tooltipTitle;
                subChart.options.plugins.tooltip.bodyColor = next.tooltipBody;
                subChart.options.plugins.tooltip.borderColor = next.tooltipBorder;
                subChart.options.scales.y.border.color = next.border;
                subChart.options.scales.y.grid.color = next.grid;
                subChart.options.scales.y.ticks.color = next.text;
                subChart.options.scales.yFlow.ticks.color = next.unsubscribed;
                subChart.options.scales.x.border.color = next.border;
                subChart.options.scales.x.ticks.color = next.text;
                subChart.update('none');
            });
        }

        var suggCanvas = document.getElementById('suggestionActivityChart');
        if (suggCanvas) {
            var suggChart = new Chart(suggCanvas.getContext('2d'), {
                data: {
                    labels: parseSeries(suggCanvas, 'labels'),
                    datasets: [
                        {
                            type: 'line',
                            label: 'Submitted',
                            data: parseSeries(suggCanvas, 'submitted'),
                            borderColor: theme.primary,
                            fill: false,
                            tension: 0.2,
                            pointRadius: 2,
                            borderWidth: 2,
                            order: 1
                        },
                        {
                            type: 'bar',
                            label: 'Accepted',
                            data: parseSeries(suggCanvas, 'accepted'),
                            backgroundColor: theme.subscribedBar,
                            borderColor: theme.subscribed,
                            borderWidth: 1,
                            borderRadius: 3,
                            maxBarThickness: 28,
                            stack: 'outcomes',
                            order: 2
                        },
                        {
                            type: 'bar',
                            label: 'Feedback given',
                            data: parseSeries(suggCanvas, 'declined'),
                            backgroundColor: theme.unsubscribedBar,
                            borderColor: theme.unsubscribed,
                            borderWidth: 1,
                            borderRadius: 3,
                            maxBarThickness: 28,
                            stack: 'outcomes',
                            order: 3
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
                            beginAtZero: true,
                            stacked: true,
                            border: { color: theme.border },
                            grid: { color: theme.grid },
                            ticks: { precision: 0, color: theme.text }
                        },
                        x: {
                            stacked: true,
                            border: { color: theme.border },
                            grid: { display: false },
                            ticks: {
                                color: theme.text,
                                autoSkip: true,
                                maxTicksLimit: 8,
                                maxRotation: 0,
                                minRotation: 0
                            }
                        }
                    }
                }
            });

            themeUpdaters.push(function (next) {
                suggChart.data.datasets[0].borderColor = next.primary;
                suggChart.data.datasets[1].backgroundColor = next.subscribedBar;
                suggChart.data.datasets[1].borderColor = next.subscribed;
                suggChart.data.datasets[2].backgroundColor = next.unsubscribedBar;
                suggChart.data.datasets[2].borderColor = next.unsubscribed;
                suggChart.options.plugins.legend.labels.color = next.text;
                suggChart.options.plugins.tooltip.backgroundColor = next.tooltipBg;
                suggChart.options.plugins.tooltip.titleColor = next.tooltipTitle;
                suggChart.options.plugins.tooltip.bodyColor = next.tooltipBody;
                suggChart.options.plugins.tooltip.borderColor = next.tooltipBorder;
                suggChart.options.scales.y.border.color = next.border;
                suggChart.options.scales.y.grid.color = next.grid;
                suggChart.options.scales.y.ticks.color = next.text;
                suggChart.options.scales.x.border.color = next.border;
                suggChart.options.scales.x.ticks.color = next.text;
                suggChart.update('none');
            });
        }

        if (colorSchemeQuery && themeUpdaters.length) {
            var handleSchemeChange = function (event) {
                var next = getChartTheme(event.matches);
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
