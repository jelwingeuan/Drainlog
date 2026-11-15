import Foundation
import SQLite3

// Rows of the tables drainlogd writes. The schema and its meaning are documented at the top
// of core/src/store.rs; times are unix seconds, booleans 0/1.

/// One row of `nights`.
struct Night: Identifiable, Hashable {
    let id: Int64  // start_ts
    let end: Int64
    let startPct: Int
    let endPct: Int
    let dropPct: Int
    let dropMah: Int
    /// true: from IOKit samples. false: coarse, from Charge %.
    let measured: Bool
    /// A cause key, "normal" or "cant_tell".
    let topCause: String
    /// Whether this night was priced with a calibrated baseline.
    let calibrated: Bool
    let headline: String
    let timeline: [TimelineItem]
}

struct TimelineItem: Decodable, Hashable {
    let ts: Int64?
    let text: String
}

/// One row of `causes`: a bucket of a night's drop.
struct Cause: Hashable {
    let key: String
    let label: String
    let mah: Int
    let seconds: Int
    let count: Int
    let estimated: Bool
    let fix: String
    let undo: String
}

/// One row of `intervals`.
struct Interval: Hashable {
    enum State: String { case asleep, darkwake, awake }
    let start: Int64
    let end: Int64
    let state: State
    /// Sleep reason ("Clamshell Sleep") for an interval entered by sleeping, else the wake reason.
    let reason: String
    let night: Int64?
}

/// One row of `samples`: an IOKit battery read.
struct Sample: Hashable {
    let ts: Int64
    let mah: Int
}

/// The `baseline` row: what the next night will be priced with.
struct Baseline: Hashable {
    let mahPerHour: Double
    let source: String
    let cleanNights: Int
    let calibrated: Bool
}

/// Everything the report needs about one night beyond its `nights` row.
struct NightDetail: Hashable {
    /// "baseline" first, attributed causes largest first, "unexplained" last.
    let causes: [Cause]
    /// The night's intervals plus the one that woke the Mac (it starts at the night's end).
    let intervals: [Interval]
    /// On-battery samples from 10 min before the night to 10 min after.
    let samples: [Sample]
}

struct StoreError: Error, CustomStringConvertible {
    let description: String
}

/// Reads drainlogd's database, read-only. The agent writes it in WAL mode, so reading never
/// blocks it.
final class Store {
    let url: URL
    private var db: OpaquePointer?

    init(url: URL = Store.defaultURL) {
        self.url = url
    }

    /// `$DRAINLOG_DB` (the name drainlogd uses too), else the agent's default path.
    static var defaultURL: URL {
        if let path = ProcessInfo.processInfo.environment["DRAINLOG_DB"], !path.isEmpty {
            return URL(filePath: path)
        }
        return URL.homeDirectory.appending(path: "Library/Application Support/Drainlog/drainlog.db")
    }

    /// False until the agent has run once.
    var exists: Bool { FileManager.default.fileExists(atPath: url.path) }

    /// Runs `body` in one read transaction, so every query sees the same rebuild of the
    /// derived tables.
    func read<T>(_ body: () throws -> T) throws -> T {
        try open()
        try exec("BEGIN")
        defer { try? exec("COMMIT") }
        return try body()
    }

    /// Newest first.
    func nights() throws -> [Night] {
        try query("""
            SELECT start_ts, end_ts, start_pct, end_pct, drop_pct, drop_mah, measured, top_cause,
                   baseline_calibrated, headline, timeline
            FROM nights ORDER BY start_ts DESC
            """) { r in
            Night(
                id: r.int64(0), end: r.int64(1), startPct: r.int(2), endPct: r.int(3),
                dropPct: r.int(4), dropMah: r.int(5), measured: r.bool(6), topCause: r.text(7),
                calibrated: r.bool(8), headline: r.text(9),
                timeline: (try? JSONDecoder().decode([TimelineItem].self, from: Data(r.text(10).utf8))) ?? []
            )
        }
    }

    func detail(_ night: Night) throws -> NightDetail {
        let causes = try query("""
            SELECT cause, label, mah, seconds, count, estimated, fix, undo
            FROM causes WHERE night = ?1 ORDER BY rowid
            """, [night.id]) { r in
            Cause(
                key: r.text(0), label: r.text(1), mah: r.int(2), seconds: r.int(3), count: r.int(4),
                estimated: r.bool(5), fix: r.text(6), undo: r.text(7))
        }
        let intervals = try query("""
            SELECT start_ts, end_ts, state, reason, night FROM intervals
            WHERE start_ts >= ?1 AND start_ts <= ?2 ORDER BY start_ts
            """, [night.id, night.end]) { r in
            Interval(
                start: r.int64(0), end: r.int64(1), state: Interval.State(rawValue: r.text(2)) ?? .awake,
                reason: r.text(3), night: r.optionalInt64(4))
        }
        // The same 10 min the core allows between a night's edge and its bounding sample.
        let samples = try query("""
            SELECT ts, mah FROM samples
            WHERE ts BETWEEN ?1 AND ?2 AND on_ac = 0 ORDER BY ts
            """, [night.id - 600, night.end + 600]) { r in
            Sample(ts: r.int64(0), mah: r.int(1))
        }
        return NightDetail(causes: causes, intervals: intervals, samples: samples)
    }

    func baseline() throws -> Baseline? {
        try query("SELECT mah_per_h, source, clean_nights, calibrated FROM baseline WHERE id = 1") { r in
            Baseline(mahPerHour: r.double(0), source: r.text(1), cleanNights: r.int(2), calibrated: r.bool(3))
        }.first
    }

    /// The first battery sample: when the agent started recording.
    func recordingSince() throws -> Date? {
        try query("SELECT MIN(ts) FROM samples") { r in
            r.optionalInt64(0).map { Date(timeIntervalSince1970: TimeInterval($0)) }
        }.first ?? nil
    }

    // MARK: SQLite

    private func open() throws {
        guard db == nil else { return }
        var handle: OpaquePointer?
        let rc = sqlite3_open_v2(url.path, &handle, SQLITE_OPEN_READONLY, nil)
        guard rc == SQLITE_OK, let handle else {
            let message = handle.map { String(cString: sqlite3_errmsg($0)) } ?? "error \(rc)"
            sqlite3_close(handle)
            throw StoreError(description: "Can't open \(url.path): \(message)")
        }
        sqlite3_busy_timeout(handle, 2000)
        db = handle
    }

    private func exec(_ sql: String) throws {
        guard sqlite3_exec(db, sql, nil, nil, nil) == SQLITE_OK else { throw error() }
    }

    private func query<T>(_ sql: String, _ args: [Int64] = [], row: (Row) throws -> T) throws -> [T] {
        try open()
        var stmt: OpaquePointer?
        guard sqlite3_prepare_v2(db, sql, -1, &stmt, nil) == SQLITE_OK, let stmt else { throw error() }
        defer { sqlite3_finalize(stmt) }
        for (i, arg) in args.enumerated() {
            sqlite3_bind_int64(stmt, Int32(i + 1), arg)
        }
        var out: [T] = []
        while true {
            switch sqlite3_step(stmt) {
            case SQLITE_ROW: out.append(try row(Row(stmt: stmt)))
            case SQLITE_DONE: return out
            default: throw error()
            }
        }
    }

    private func error() -> StoreError {
        StoreError(description: db.map { String(cString: sqlite3_errmsg($0)) } ?? "no database")
    }
}

/// The current row of a stepped statement.
private struct Row {
    let stmt: OpaquePointer

    func int64(_ i: Int32) -> Int64 { sqlite3_column_int64(stmt, i) }
    func int(_ i: Int32) -> Int { Int(sqlite3_column_int64(stmt, i)) }
    func double(_ i: Int32) -> Double { sqlite3_column_double(stmt, i) }
    func bool(_ i: Int32) -> Bool { sqlite3_column_int64(stmt, i) != 0 }
    func text(_ i: Int32) -> String { sqlite3_column_text(stmt, i).map { String(cString: $0) } ?? "" }
    func optionalInt64(_ i: Int32) -> Int64? {
        sqlite3_column_type(stmt, i) == SQLITE_NULL ? nil : sqlite3_column_int64(stmt, i)
    }
}
