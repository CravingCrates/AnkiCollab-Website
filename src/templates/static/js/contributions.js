/* The site's CSP forbids inline <script>, so behaviour lives here and the chart's numbers arrive on
 * the canvas as data-* attributes. */
$(function () {
    var timeline = $('#timelineOverview');
    if (timeline.length) {
        timeline.DataTable({ pageLength: 10, order: [[0, 'desc']], searching: false });
    }

    var contributors = $('#contributorsOverview');
    if (contributors.length) {
        contributors.DataTable({ pageLength: 25, order: [] });
    }

    var suggestions = $('#suggestionOverview');
    if (suggestions.length) {
        suggestions.DataTable({ pageLength: 25, order: [] });
    }

    var canvas = document.getElementById('subscriberTimelineChart');
    if (!canvas || typeof Chart === 'undefined') {
        return;
    }

    function series(name) {
        var raw = canvas.getAttribute('data-' + name);
        if (!raw) {
            return [];
        }
        try {
            return JSON.parse(raw) || [];
        } catch (err) {
            return [];
        }
    }

    // The Active filter changes which series is the primary one; the other two stay put.
    var tracked = canvas.getAttribute('data-tracked') === 'true';
    var view = canvas.getAttribute('data-view');
    var primary = series('series');
    var primaryLabel = 'Subscribers';
    if (tracked && view === '30') {
        primary = series('active-30d');
        primaryLabel = 'Active in the last 30 days';
    } else if (tracked && view === '90') {
        primary = series('active-90d');
        primaryLabel = 'Active in the last 90 days';
    }

    function getChartTheme(isDark) {
        if (isDark) {
            return {
                primary: '#60a5fa',
                subscribed: '#4ade80',
                unsubscribed: '#94a3b8',
                text: '#cbd5e1',
                grid: 'rgba(255, 255, 255, 0.10)',
                border: 'rgba(255, 255, 255, 0.16)',
                tooltipBg: '#242428',
                tooltipTitle: '#f5f5f7',
                tooltipBody: '#cbd5e1',
                tooltipBorder: '#38383d'
            };
        }
        return {
            primary: '#4a6fd4',
            subscribed: '#28a745',
            unsubscribed: '#adb5bd',
            text: '#475569',
            grid: 'rgba(0, 0, 0, 0.1)',
            border: 'rgba(0, 0, 0, 0.1)',
            tooltipBg: 'rgba(0, 0, 0, 0.8)',
            tooltipTitle: '#ffffff',
            tooltipBody: '#ffffff',
            tooltipBorder: 'rgba(0, 0, 0, 0)'
        };
    }

    var colorSchemeQuery = window.matchMedia
        ? window.matchMedia('(prefers-color-scheme: dark)')
        : null;
    var theme = getChartTheme(Boolean(colorSchemeQuery && colorSchemeQuery.matches));

    var chart = new Chart(canvas.getContext('2d'), {
        type: 'line',
        data: {
            labels: series('labels'),
            datasets: [
                {
                    label: primaryLabel,
                    data: primary,
                    borderColor: theme.primary,
                    fill: false,
                    tension: 0,
                    pointRadius: 0,
                    borderWidth: 2
                },
                {
                    label: 'New subscribers',
                    data: series('new'),
                    borderColor: theme.subscribed,
                    fill: false,
                    tension: 0,
                    pointRadius: 0,
                    borderWidth: 1
                },
                {
                    label: 'Unsubscribed',
                    data: series('unsubscribed'),
                    borderColor: theme.unsubscribed,
                    fill: false,
                    tension: 0,
                    pointRadius: 0,
                    borderWidth: 1,
                    borderDash: [3, 3]
                }
            ]
        },
        options: {
            responsive: true,
            maintainAspectRatio: false,
            animation: false,
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
                    border: { color: theme.border },
                    grid: { color: theme.grid },
                    ticks: { precision: 0, color: theme.text }
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

    function applyChartTheme(isDark) {
        var next = getChartTheme(isDark);
        chart.data.datasets[0].borderColor = next.primary;
        chart.data.datasets[1].borderColor = next.subscribed;
        chart.data.datasets[2].borderColor = next.unsubscribed;
        chart.options.plugins.legend.labels.color = next.text;
        chart.options.plugins.tooltip.backgroundColor = next.tooltipBg;
        chart.options.plugins.tooltip.titleColor = next.tooltipTitle;
        chart.options.plugins.tooltip.bodyColor = next.tooltipBody;
        chart.options.plugins.tooltip.borderColor = next.tooltipBorder;
        chart.options.scales.y.border.color = next.border;
        chart.options.scales.y.grid.color = next.grid;
        chart.options.scales.y.ticks.color = next.text;
        chart.options.scales.x.border.color = next.border;
        chart.options.scales.x.ticks.color = next.text;
        chart.update('none');
    }

    if (colorSchemeQuery) {
        var handleSchemeChange = function (event) {
            applyChartTheme(event.matches);
        };
        if (typeof colorSchemeQuery.addEventListener === 'function') {
            colorSchemeQuery.addEventListener('change', handleSchemeChange);
        } else if (typeof colorSchemeQuery.addListener === 'function') {
            colorSchemeQuery.addListener(handleSchemeChange);
        }
    }
});
