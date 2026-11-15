import Charts
import SwiftUI

/// The night as state bands (asleep, DarkWake, awake) with the measured battery line on top.
/// DarkWakes also get a solid tick in a lane along the bottom, so they read by shape, not only
/// by colour.
struct NightChart: View {
    enum Style { case full, compact }

    static let asleep = Color.indigo.opacity(0.13)
    static let darkWake = Color.orange.opacity(0.2)
    static let awake = Color.teal.opacity(0.75)

    let night: Night
    let detail: NightDetail
    let style: Style
    @State private var width: CGFloat = 600
    @ScaledMetric(relativeTo: .body) private var fullHeight: CGFloat = 150
    @ScaledMetric(relativeTo: .body) private var compactHeight: CGFloat = 78

    var body: some View {
        let full = style == .full
        let scale = ChartScale(night: night, samples: detail.samples, lane: full ? 0.09 : 0.11)
        let spans = detail.intervals.filter { $0.night == night.id && $0.state != .asleep }
        let longestAwake = spans.filter { $0.state == .awake }.max { $0.end - $0.start < $1.end - $1.start }
        // The narrowest a band or tick is drawn, 2 pt, in seconds: most DarkWakes last under a minute.
        let narrowest = scale.x.upperBound.timeIntervalSince(scale.x.lowerBound) * 2 / max(width, 1)
        let ticks = full ? scale.hourTicks : [scale.night.lowerBound, scale.night.upperBound]

        Chart {
            RectangleMark(xStart: .value("Start", Fmt.date(night.id)), xEnd: .value("End", Fmt.date(night.end)))
                .foregroundStyle(Self.asleep)
            ForEach(spans, id: \.start) { iv in
                let start = Fmt.date(iv.start)
                let end = max(Fmt.date(iv.end), start.addingTimeInterval(narrowest))
                if iv.state == .darkwake {
                    RectangleMark(xStart: .value("Start", start), xEnd: .value("End", end))
                        .foregroundStyle(Self.darkWake)
                    RectangleMark(
                        xStart: .value("Start", start), xEnd: .value("End", end),
                        yStart: .value("mAh", scale.y.lowerBound), yEnd: .value("mAh", scale.laneTop))
                        .foregroundStyle(Color.orange)
                } else {
                    RectangleMark(xStart: .value("Start", start), xEnd: .value("End", end))
                        .foregroundStyle(Self.awake)
                        .annotation(position: .overlay, alignment: .top) {
                            if full && iv == longestAwake { AwakeLabel() }
                        }
                }
            }
            if full {
                ForEach(scale.grid, id: \.self) { mah in
                    RuleMark(y: .value("mAh", mah))
                        .foregroundStyle(.separator)
                        .lineStyle(StrokeStyle(lineWidth: 1))
                        .annotation(position: .top, alignment: .trailing, spacing: 2) {
                            Text(Fmt.mah(mah)).font(.caption2).foregroundStyle(.supporting)
                        }
                }
            }
            ForEach(detail.samples, id: \.ts) { s in
                LineMark(x: .value("Time", Fmt.date(s.ts)), y: .value("mAh", s.mah))
                    .foregroundStyle(Color.primary)
                    .lineStyle(StrokeStyle(lineWidth: full ? 1.8 : 1.6, lineCap: .round, lineJoin: .round))
            }
            if detail.samples.count > 1, let first = detail.samples.first, let last = detail.samples.last {
                ForEach([first, last], id: \.ts) { s in
                    PointMark(x: .value("Time", Fmt.date(s.ts)), y: .value("mAh", s.mah))
                        .foregroundStyle(Color.primary)
                        .symbolSize(full ? 30 : 20)
                }
            }
        }
        .chartXScale(domain: scale.x)
        .chartYScale(domain: scale.y)
        .chartXAxis {
            AxisMarks(values: ticks) { value in
                AxisValueLabel(anchor: value.index == 0 ? .topLeading : value.index == value.count - 1 ? .topTrailing : .top) {
                    if let date = value.as(Date.self) {
                        Text(date.formatted(Date.FormatStyle(locale: Fmt.locale).hour().minute()))
                            .foregroundStyle(.supporting)
                    }
                }
            }
        }
        .chartYAxis(.hidden)
        .chartLegend(.hidden)
        .chartPlotStyle { $0.clipShape(.rect(cornerRadius: full ? 6 : 4)) }
        .frame(height: full ? fullHeight : compactHeight)
        .onGeometryChange(for: CGFloat.self) { $0.size.width } action: { width = $0 }
        .accessibilityElement(children: .ignore)
        .accessibilityLabel("Night timeline")
        .accessibilityValue(summary)
        .accessibilityChartDescriptor(
            NightChartDescriptor(summary: summary, x: scale.x, y: scale.y, samples: detail.samples))
    }

    /// What the chart shows, in a sentence, for VoiceOver.
    private var summary: String {
        let t = NightTotals(night, detail.intervals)
        var parts = ["From \(Fmt.clock(night.id)) to \(Fmt.clock(night.end)): asleep \(Fmt.duration(t.asleep))"]
        if t.darkWake > 0 {
            parts.append(t.wakes > 0
                ? "\(Fmt.plural(t.wakes, "DarkWake")) over \(Fmt.duration(t.darkWake))"
                : "DarkWake \(Fmt.duration(t.darkWake))")
        }
        if t.awake > 0 { parts.append("awake \(Fmt.duration(t.awake))") }
        var text = parts.joined(separator: ", ") + "."
        if let first = detail.samples.first, let last = detail.samples.last, detail.samples.count > 1 {
            text += " Battery went from \(Fmt.spokenMah(first.mah)) to \(Fmt.spokenMah(last.mah)), measured."
        }
        return text
    }
}

private struct AwakeLabel: View {
    var body: some View {
        Text("Awake")
            .font(.caption.weight(.semibold))
            .padding(.horizontal, 4)
            .background(.background, in: .rect(cornerRadius: 4))
            .overlay(RoundedRectangle(cornerRadius: 4).strokeBorder(.separator, lineWidth: 0.5))
            .padding(.top, 3)
            .fixedSize()
    }
}

/// The chart's axes. Pure arithmetic, so it can be tested.
struct ChartScale {
    let night: ClosedRange<Date>
    let x: ClosedRange<Date>
    let y: ClosedRange<Double>
    /// The top of the DarkWake tick lane, in mAh.
    let laneTop: Double
    /// Round mAh values for up to three grid lines, clear of the top and bottom edges.
    let grid: [Int]

    init(night: Night, samples: [Sample], lane: Double) {
        self.night = Fmt.date(night.id)...Fmt.date(night.end)
        // Wide enough for the bounding samples, which may be taken up to 10 min outside the night.
        let first = min(self.night.lowerBound, Fmt.date(samples.first?.ts ?? night.id))
        x = first...max(self.night.upperBound, Fmt.date(samples.last?.ts ?? night.end))
        guard samples.count > 1, let lo = samples.map(\.mah).min(), let hi = samples.map(\.mah).max() else {
            y = 0...1
            laneTop = lane
            grid = []
            return
        }
        let range = Double(max(hi - lo, 20))
        y = (Double(lo) - range * 0.3)...(Double(hi) + range * 0.3)
        let height = y.upperBound - y.lowerBound
        laneTop = y.lowerBound + height * lane
        let band = (y.lowerBound + height * 0.12)...(y.upperBound - height * 0.18)
        // The finest round step (10, 20, 25, 50, 100, … mAh) that gives at most three lines.
        let steps = [1.0, 10, 100, 1000].flatMap { power in [10.0, 20, 25, 50].map { $0 * power } }
        grid = steps.lazy
            .map { step in
                Array(stride(from: (band.lowerBound / step).rounded(.up) * step, through: band.upperBound, by: step))
            }
            .first { $0.count <= 3 }?
            .map { Int($0) } ?? []
    }

    /// The night's start and end, and every second full hour between them that is at least
    /// 90 min from the start and an hour from the end: 23:10, 01:00, 03:00, 05:00, 07:20.
    var hourTicks: [Date] {
        let (start, end) = (night.lowerBound, night.upperBound)
        guard let first = Calendar.current.nextDate(
            after: start.addingTimeInterval(5400 - 1), matching: DateComponents(minute: 0, second: 0),
            matchingPolicy: .nextTime)
        else { return [start, end] }
        let middle = stride(from: first, through: end.addingTimeInterval(-3600), by: 7200)
        return [start] + middle + [end]
    }
}

/// Lets VoiceOver read the battery line as an audio graph.
struct NightChartDescriptor: AXChartDescriptorRepresentable {
    let summary: String
    let x: ClosedRange<Date>
    let y: ClosedRange<Double>
    let samples: [Sample]

    func makeChartDescriptor() -> AXChartDescriptor {
        let time = AXNumericDataAxisDescriptor(
            title: "Time",
            range: x.lowerBound.timeIntervalSince1970...x.upperBound.timeIntervalSince1970,
            gridlinePositions: []
        ) { Fmt.clock(Int64($0)) }
        let charge = AXNumericDataAxisDescriptor(title: "Battery charge", range: y, gridlinePositions: []) {
            Fmt.spokenMah(Int($0))
        }
        let battery = AXDataSeriesDescriptor(
            name: "Battery, measured", isContinuous: true,
            dataPoints: samples.map { AXDataPoint(x: Double($0.ts), y: Double($0.mah)) })
        return AXChartDescriptor(
            title: "Night timeline", summary: summary, xAxis: time, yAxis: charge, additionalAxes: [],
            series: [battery])
    }
}
