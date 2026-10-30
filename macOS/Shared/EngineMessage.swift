import Foundation

enum MessageArgument: Codable, Equatable {
    case text(String), number(UInt64)
    private enum Keys: String, CodingKey { case type, value }
    init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy:Keys.self)
        switch try container.decode(String.self,forKey:.type) {
        case "text": self = .text(try container.decode(String.self,forKey:.value))
        case "number": self = .number(try container.decode(UInt64.self,forKey:.value))
        default: throw DecodingError.dataCorruptedError(forKey:.type,in:container,debugDescription:"Unknown message argument")
        }
    }
    func encode(to encoder: Encoder) throws {
        var container = encoder.container(keyedBy:Keys.self)
        switch self {
        case .text(let value): try container.encode("text",forKey:.type); try container.encode(value,forKey:.value)
        case .number(let value): try container.encode("number",forKey:.type); try container.encode(value,forKey:.value)
        }
    }
}

/// Hosts transport stable codes and raw detail; only the foreground app renders text.
struct EngineMessage: Codable, Equatable, LocalizedError {
    let code: String
    var arguments: [MessageArgument] = []
    var diagnostic: String? = nil
    private static let inputCodes: Set<String> = ["input.a_file_already_exists_at_the_new_location", "input.a_torrent_file_points_through_a_symbolic_link", "input.add_a_window_before_enabling_the_schedule", "input.choose_a_folder_outside_seedhop_s_library_state", "input.choose_a_supported_peer_encryption_policy", "input.choose_an_existing_download_folder", "input.choose_an_existing_file", "input.choose_at_least_one_day_and_valid_schedule", "input.choose_low_normal_or_high_for_every_file", "input.files_already_exist_here_enable_verification_of_existing", "input.paste_a_valid_magnet_link", "input.refresh_the_recovery_notice_and_try_again", "input.schedule_windows_overlap_adjust_their_days_or_times", "input.select_at_least_one_file_to_download", "input.select_valid_files", "input.set_an_alternative_download_or_upload_limit_before", "input.the_recovery_record_changed_refresh_and_try_again", "input.the_torrent_contains_an_unsafe_file_path", "input.the_torrent_metadata_exceeds_mb", "input.there_is_no_pending_cleanup", "input.there_is_no_pending_file_cleanup", "input.this_preview_accepts_magnet_links_or_torrent_files", "input.this_request_is_already_resolving", "input.this_torrent_is_already_in_your_library", "input.this_torrent_is_no_longer_in_the_library", "input.two_files_cannot_share_a_path_or_contain", "input.unknown_details_section", "input.use_at_most_schedule_windows"]
    var errorDescription: String? { displayText }
    var displayText: String {
        let summary = localizedText
        guard let diagnostic, !diagnostic.isEmpty, diagnostic != summary,
              !Self.inputCodes.contains(code), code != "engine.busy", code != "engine.stopping", code != "torrent.unsupported" else { return summary }
        return summary + "\n\n" + String(localized:"Technical details: \(diagnostic)",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)
    }
    var localizedText: String {
        switch code {
        case "engine.busy": return String(localized:"Another SeedHop engine is already using this library.",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)
        case "engine.stopping": return String(localized:"The engine is stopping. Try again once it reconnects.",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)
        case "engine.failed": return String(localized:"The engine could not complete this action. Check the details, then try again.",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)
        case "input.invalid": return String(localized:"Check the entered values and try again.",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)
        case "torrent.unsupported": return String(localized:"This torrent format is not supported. Use a BitTorrent v1, v2, or hybrid torrent.",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)
        case "operation.cancelled": return String(localized:"The operation was cancelled.",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)
        case "library.corrupt": return String(localized:"The saved library could not be read. Your files have been preserved.",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)
        case "storage.space": return String(localized:"Not enough disk space",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)
        case "storage.permission": return String(localized:"File access is restricted",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)
        case "storage.missing": return String(localized:"Files or destination unavailable",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)
        case "storage.failed": return String(localized:"Check the destination disk and folder, then retry.",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)
        case "statistics.save_failed": return String(localized:"Transfer statistics could not be saved. Check space and access to the library folder. Totals remain in memory until saving succeeds.",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)
        case "schedule.apply_failed": return String(localized:"The weekly schedule could not be applied. Check the details and transfer settings.",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)
        case "recovery.new": return String(localized:"The new location is saved. Extra copies were retained. Retry cleanup after restoring access, or inspect the folders and keep the extra copies.",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)
        case "recovery.original": return String(localized:"The original location is still active. Extra copies were retained. Retry cleanup after restoring access, or inspect the folders and keep the extra copies.",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)
        case "recovery.unknown": return String(localized:"The active location could not be established. All copies were retained. Inspect the folders before changing files.",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)
        case "host.version_mismatch": return String(localized:"Restart SeedHop to finish updating its engine.",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)
        case "host.stopping": return String(localized:"The engine is stopping. Please wait.",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)
        case "host.background_save": return String(localized:"Could not save the background preference.",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)
        case "host.encryption": return String(localized:"Choose a supported peer encryption policy.",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)
        case "host.schedule": return String(localized:"Provide schedule settings.",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)
        case "host.bandwidth": return String(localized:"Missing alternative bandwidth limits.",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)
        case "host.unsupported": return String(localized:"This action is not supported by this engine version.",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)
        case "input.a_file_already_exists_at_the_new_location": return String(localized:"A file already exists at the new location. Choose another name or folder.",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)
        case "input.a_torrent_file_points_through_a_symbolic_link": return String(localized:"A torrent file points through a symbolic link. Choose another folder.",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)
        case "input.add_a_window_before_enabling_the_schedule": return String(localized:"Add a window before enabling the schedule.",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)
        case "input.choose_a_folder_outside_seedhop_s_library_state": return String(localized:"Choose a folder outside SeedHop's library state.",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)
        case "input.choose_a_supported_peer_encryption_policy": return String(localized:"Choose a supported peer encryption policy.",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)
        case "input.choose_an_existing_download_folder": return String(localized:"Choose an existing download folder.",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)
        case "input.choose_an_existing_file": return String(localized:"Choose an existing file.",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)
        case "input.choose_at_least_one_day_and_valid_schedule": return String(localized:"Choose at least one day and valid schedule times.",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)
        case "input.choose_low_normal_or_high_for_every_file": return String(localized:"Choose Low, Normal, or High for every file.",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)
        case "input.files_already_exist_here_enable_verification_of_existing": return String(localized:"Files already exist here. Enable verification of existing files or choose another folder.",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)
        case "input.paste_a_valid_magnet_link": return String(localized:"Paste a valid magnet link.",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)
        case "input.refresh_the_recovery_notice_and_try_again": return String(localized:"Refresh the recovery notice and try again.",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)
        case "input.schedule_windows_overlap_adjust_their_days_or_times": return String(localized:"Schedule windows overlap. Adjust their days or times.",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)
        case "input.select_at_least_one_file_to_download": return String(localized:"Select at least one file to download.",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)
        case "input.select_valid_files": return String(localized:"Select valid files.",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)
        case "input.set_an_alternative_download_or_upload_limit_before": return String(localized:"Set an alternative download or upload limit before enabling alternative limits.",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)
        case "input.the_recovery_record_changed_refresh_and_try_again": return String(localized:"The recovery record changed. Refresh and try again.",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)
        case "input.the_torrent_contains_an_unsafe_file_path": return String(localized:"The torrent contains an unsafe file path.",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)
        case "input.the_torrent_metadata_exceeds_mb": return String(localized:"The torrent metadata exceeds 32 MB.",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)
        case "input.there_is_no_pending_cleanup": return String(localized:"There is no pending cleanup.",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)
        case "input.there_is_no_pending_file_cleanup": return String(localized:"There is no pending file cleanup.",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)
        case "input.this_preview_accepts_magnet_links_or_torrent_files": return String(localized:"This preview accepts magnet links or .torrent files.",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)
        case "input.this_request_is_already_resolving": return String(localized:"This request is already resolving.",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)
        case "input.this_torrent_is_already_in_your_library": return String(localized:"This torrent is already in your library.",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)
        case "input.this_torrent_is_no_longer_in_the_library": return String(localized:"This torrent is no longer in the library.",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)
        case "input.two_files_cannot_share_a_path_or_contain": return String(localized:"Two files cannot share a path or contain one another.",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)
        case "input.unknown_details_section": return String(localized:"Unknown details section",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)
        case "input.use_at_most_schedule_windows":
            let maximum: UInt64
            if case .number(let value) = arguments.first { maximum = value } else { maximum = 32 }
            return String(localized:"Use at most \(maximum) schedule windows.",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)
        default: return String(localized:"SeedHop could not complete this action. Check the details, then try again.",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)
        }
    }
}
