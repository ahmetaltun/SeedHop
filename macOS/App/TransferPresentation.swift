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
    var label: String { connected ? currentLabel : String(localized:"Last: \(currentLabel)",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale) }
    private var currentLabel: String {
        if let change = fileChange {
            if change.cancelRequested { return String(localized:"Cancelling",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale) }
            switch change.phase {
            case "copying": return change.kind == "rename" ? String(localized:"Renaming file",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale) : String(localized:"Moving files",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)
            case "saving": return String(localized:"Saving location",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)
            case "cleaning": return String(localized:"Cleaning up",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)
            case "restoring": return String(localized:"Restoring files",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)
            case "verifying": return String(localized:"Starting check",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)
            default: return String(localized:"Preparing files",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)
            }
        }
        if hasRecovery { return String(localized:"Cleanup needed",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale) }
        if torrent.state == "queued" && torrent.queuePosition > 0 { return String(localized:"Queued · \(torrent.queuePosition)",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale) }
        if isError && (torrent.errorKind ?? "").hasPrefix("storage") { return String(localized:"File access error",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale) }
        if torrent.state == "paused" && !torrent.isPaused { return String(localized:"Preparing transfer",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale) }
        return torrent.statusLabel
    }
    var accessibleLabel: String {
        if isQueued && torrent.queuePosition > 0 {
            return connected ? String(localized:"Queued, position \(torrent.queuePosition)",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale) : String(localized:"Last known: Queued, position \(torrent.queuePosition)",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)
        }
        return label
    }
    var detail: String {
        if !connected { return String(localized:"This is the last known state. Reconnect to see current activity.",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale) }
        if let change = fileChange { return change.detail }
        if hasRecovery { return String(localized:"\(torrent.statusLabel). Review the retained-file cleanup above before changing files.",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale) }
        if isError { return errorGuidance }
        switch torrent.state {
        case "scheduled": return String(localized:"Suspended by the weekly schedule. Manual pause choices are preserved. Change scheduling in Transfers settings.",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)
        case "queued": return torrent.queuePosition > 0 ? String(localized:"Position \(torrent.queuePosition) in the download queue. Waiting for an active download slot.",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale) : String(localized:"Waiting for an active download slot.",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)
        case "waiting": return torrent.peers == 0 ? String(localized:"No peers are connected. Available discovery methods depend on the torrent and current network.",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale) : String(localized:"\(torrent.peers) peers connected; this snapshot shows no download activity.",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)
        case "checking": return torrent.isPaused ? String(localized:"Checking existing files. This transfer will remain paused afterward.",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale) : String(localized:"Checking existing files before continuing the transfer.",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)
        case "paused": return torrent.isPaused ? String(localized:"Paused by you. Resume when you are ready.",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale) : String(localized:"Preparing the transfer without changing your saved pause setting.",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)
        case "seeding": return String(localized:"Your selected files are complete and available to share.",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)
        case "downloading": return String(localized:"Receiving data for your selected files.",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)
        default: return String(localized:"Waiting for the next engine update.",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)
        }
    }
    var errorTitle: String {
        switch torrent.errorKind {
        case "storage_space": return String(localized:"Not enough disk space",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)
        case "storage_permission": return String(localized:"File access is restricted",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)
        case "storage_missing": return String(localized:"Files or destination unavailable",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)
        case "storage": return String(localized:"Files need attention",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)
        default: return String(localized:"Transfer stopped",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)
        }
    }
    private var errorGuidance: String {
        let continuation = torrent.isPaused ? String(localized:"Retry checks existing files and leaves this transfer paused.",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale) : String(localized:"Retry checks existing files and follows the download queue.",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)
        switch torrent.errorKind {
        case "storage_space": return String(localized:"Free space on the destination disk, then retry. \(continuation)",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)
        case "storage_permission": return String(localized:"Restore read and write access to the saved folder, then retry. \(continuation)",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)
        case "storage_missing": return String(localized:"Reconnect the drive or restore the saved path, then retry. SeedHop will keep the saved location. \(continuation)",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)
        case "storage": return String(localized:"Check the destination disk and folder, then retry. \(continuation)",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)
        default: return String(localized:"The engine stopped this transfer. \(continuation) See details if the error returns.",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)
        }
    }
    var progressTitle: String { fileChange != nil ? String(localized:"Copied",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale) : isChecking ? String(localized:"Checked",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale) : String(localized:"Downloaded",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale) }
    var progressMetricTitle: String { fileChange != nil ? String(localized:"Copied bytes",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale) : isChecking ? String(localized:"Checked bytes",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale) : String(localized:"Downloaded bytes",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale) }
    var completedBytes: UInt64 { fileChange?.completedBytes ?? torrent.downloadedBytes }
    var totalBytes: UInt64 { fileChange?.totalBytes ?? torrent.totalBytes }
    var progress: Double? {
        if isError { return nil }
        if let change = fileChange, change.phase != "copying" || change.totalBytes == 0 { return nil }
        return totalBytes == 0 ? 0 : min(1, Double(completedBytes) / Double(totalBytes))
    }
    var progressDescription: String {
        guard let progress else { return String(localized:"Progress is unavailable during this operation.",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale) }
        let percentage = LocaleText.percent(progress)
        if fileChange != nil { return String(localized:"\(percentage) copied",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale) }
        return isChecking ? String(localized:"\(percentage) checked",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale) : String(localized:"\(percentage) downloaded",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)
    }
}
