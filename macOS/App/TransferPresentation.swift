import Foundation

/// One presentation of snapshot facts, shared by rows, filters, and inspector.
/// Quiet peers are not evidence of a tracker failure or an unavailable network.
struct TransferPresentation {
    let torrent: TorrentItem
    var operation: FileOperationItem? = nil
    var recovery: StorageRecoveryItem? = nil
    var connected = true

    private var fileChange: FileOperationItem? { operation?.torrentId == torrent.id ? operation : nil }
    var hasRecovery: Bool { recovery?.torrentId == torrent.id }
    var needsAttention: Bool { (fileChange == nil && torrent.state == "error") || hasRecovery }
    var isChecking: Bool { fileChange == nil && torrent.state == "checking" }
    var isError: Bool { fileChange == nil && torrent.state == "error" }
    var isQueued: Bool { fileChange == nil && torrent.state == "queued" }
    var canRetry: Bool { isError && !(hasRecovery || recovery?.torrentId.isEmpty == true) }
    var canMoveAfterError: Bool { isError && recovery == nil && ["storage_space", "storage_permission", "storage"].contains(torrent.errorKind ?? "") }
    var isActive: Bool {
        torrent.isActive || (fileChange != nil && !torrent.isPaused && torrent.progress < 1)
    }
    var isSeeding: Bool {
        torrent.state == "seeding" || (fileChange != nil && !torrent.isPaused && torrent.progress >= 1)
    }
    var colorState: String {
        if !connected { return "paused" }
        if fileChange != nil { return "downloading" }
        if needsAttention { return "error" }
        return torrent.state
    }
    var label: String { connected ? currentLabel : "Last: \(currentLabel)" }
    private var currentLabel: String {
        if let change = fileChange {
            if change.cancelRequested { return "Cancelling" }
            switch change.phase {
            case "copying": return change.kind == "rename" ? "Renaming file" : "Moving files"
            case "saving": return "Saving location"
            case "cleaning": return "Cleaning up"
            case "restoring": return "Restoring files"
            case "verifying": return "Starting check"
            default: return "Preparing files"
            }
        }
        if hasRecovery { return "Cleanup needed" }
        if torrent.state == "queued" && torrent.queuePosition > 0 { return "Queued · \(torrent.queuePosition)" }
        if isError && (torrent.errorKind ?? "").hasPrefix("storage") { return "File access error" }
        if torrent.state == "paused" && !torrent.isPaused { return "Preparing transfer" }
        return torrent.statusLabel
    }
    var accessibleLabel: String {
        if isQueued && torrent.queuePosition > 0 { return "\(connected ? "" : "Last known: ")Queued, position \(torrent.queuePosition)" }
        return label
    }
    var detail: String {
        if !connected { return "This is the last known state. Reconnect to see current activity." }
        if let change = fileChange { return change.detail }
        if hasRecovery { return "\(torrent.statusLabel). Review the retained-file cleanup above before changing files." }
        if isError { return errorGuidance }
        switch torrent.state {
        case "queued": return torrent.queuePosition > 0 ? "Position \(torrent.queuePosition) in the download queue. Waiting for an active download slot." : "Waiting for an active download slot."
        case "waiting": return torrent.peers == 0 ? "No peers are connected. Available discovery methods depend on the torrent and current network." : "\(torrent.peers) peers connected; this snapshot shows no download activity."
        case "checking": return torrent.isPaused ? "Checking existing files. This transfer will remain paused afterward." : "Checking existing files before continuing the transfer."
        case "paused": return torrent.isPaused ? "Paused by you. Resume when you are ready." : "Preparing the transfer without changing your saved pause setting."
        case "seeding": return "Your selected files are complete and available to share."
        case "downloading": return "Receiving data for your selected files."
        default: return "Waiting for the next engine update."
        }
    }
    var errorTitle: String {
        switch torrent.errorKind {
        case "storage_space": return "Not enough disk space"
        case "storage_permission": return "File access is restricted"
        case "storage_missing": return "Files or destination unavailable"
        case "storage": return "Files need attention"
        default: return "Transfer stopped"
        }
    }
    private var errorGuidance: String {
        let continuation = torrent.isPaused ? "Retry checks existing files and leaves this transfer paused." : "Retry checks existing files and follows the download queue."
        switch torrent.errorKind {
        case "storage_space": return "Free space on the destination disk, then retry. \(continuation)"
        case "storage_permission": return "Restore read and write access to the saved folder, then retry. \(continuation)"
        case "storage_missing": return "Reconnect the drive or restore the saved path, then retry. SeedHop will keep the saved location. \(continuation)"
        case "storage": return "Check the destination disk and folder, then retry. \(continuation)"
        default: return "The engine stopped this transfer. \(continuation) See details if the error returns."
        }
    }
    var progressTitle: String { fileChange != nil ? "Copied" : isChecking ? "Checked" : "Downloaded" }
    var completedBytes: UInt64 { fileChange?.completedBytes ?? torrent.downloadedBytes }
    var totalBytes: UInt64 { fileChange?.totalBytes ?? torrent.totalBytes }
    var progress: Double? {
        if isError { return nil }
        if let change = fileChange, change.phase != "copying" || change.totalBytes == 0 { return nil }
        return totalBytes == 0 ? 0 : min(1, Double(completedBytes) / Double(totalBytes))
    }
    var progressDescription: String {
        guard let progress else { return "Progress is unavailable during this operation." }
        return "\(Int(progress * 100)) percent \(progressTitle.lowercased())"
    }
}
