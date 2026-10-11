/* The site's CSP forbids inline <script>, so behaviour lives here and the chart's numbers arrive on
 * the canvas as data-* attributes. */
$(function () {
    var timeline = $('#timelineOverview');
    if (timeline.length) {
        timeline.DataTable({ pageLength: 10, order: [], searching: false });
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

    new Chart(canvas.getContext('2d'), {
        type: 'line',
        data: {
            labels: series('labels'),
            datasets: [
                {
                    label: primaryLabel,
                    data: primary,
                    borderColor: '#4a6fd4',
                    fill: false,
                    lineTension: 0,
                    pointRadius: 0,
                    borderWidth: 2
                },
                {
                    label: 'New subscribers',
                    data: series('new'),
                    borderColor: '#28a745',
                    fill: false,
                    lineTension: 0,
                    pointRadius: 0,
                    borderWidth: 1
                },
                {
                    label: 'Unsubscribed',
                    data: series('unsubscribed'),
                    borderColor: '#adb5bd',
                    fill: false,
                    lineTension: 0,
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
            legend: {
                position: 'bottom',
                labels: { boxWidth: 12, fontSize: 11, usePointStyle: true }
            },
            scales: {
                yAxes: [{ ticks: { beginAtZero: true, precision: 0 } }],
                xAxes: [
                    {
                        gridLines: { display: false },
                        ticks: {
                            autoSkip: true,
                            maxTicksLimit: 8,
                            maxRotation: 0,
                            minRotation: 0
                        }
                    }
                ]
            }
        }
    });
});
