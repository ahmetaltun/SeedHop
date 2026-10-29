import SwiftUI

/// Draft text remains local until the engine validates and durably saves it.
struct SchedulingSettingsView: View {
    @EnvironmentObject var store: LibraryStore
    @State private var enabled = false
    @State private var windows: [ScheduleDraft] = []
    @State private var message: String?
    private let days = [("Mon", "Monday"), ("Tue", "Tuesday"), ("Wed", "Wednesday"), ("Thu", "Thursday"), ("Fri", "Friday"), ("Sat", "Saturday"), ("Sun", "Sunday")]

    var body: some View {
        VStack(alignment:.leading,spacing:14) {
            Toggle("Enable weekly schedule",isOn:$enabled).toggleStyle(.switch).controlSize(.small)
            Text("Windows use this Mac’s local time. Outside them, normal bandwidth limits and queue rules apply. Scheduling needs a running engine; it does not launch or wake the Mac.").font(.system(size:11)).foregroundStyle(.secondary)
            if let warning = store.schedulingWarning { Text(warning).font(.system(size:11)).foregroundStyle(.orange) }
            Text(store.scheduleActive ? "A schedule window is active." : "Normal transfer settings are active.").font(.system(size:11,weight:.medium))
            Text("Overnight windows end the following day. Equal start and end times mean 24 hours. Windows cannot overlap.").font(.system(size:10)).foregroundStyle(.secondary)
            ForEach($windows) { $window in
                VStack(alignment:.leading,spacing:10) {
                    HStack {
                        Text("Window \((windows.firstIndex(where:{$0.id == window.id}) ?? 0) + 1)").font(.system(size:12,weight:.semibold))
                        Spacer()
                        Button("Remove") { let id = window.id; windows.removeAll { $0.id == id } }.accessibilityLabel("Remove schedule window \((windows.firstIndex(where:{$0.id == window.id}) ?? 0) + 1)")
                    }
                    HStack(spacing:5) {
                        ForEach(0..<7,id:\.self) { day in
                            Toggle(days[day].0,isOn:Binding(get:{window.days & (1 << day) != 0},set:{ selected in
                                if selected { window.days |= 1 << day } else { window.days &= ~(1 << day) }
                            })).toggleStyle(.button).accessibilityLabel(days[day].1)
                        }
                    }
                    HStack {
                        Text("From"); TextField("09:00",text:$window.start).frame(width:65).accessibilityLabel("Window start, 24-hour HH:mm")
                        Text("to"); TextField("17:00",text:$window.end).frame(width:65).accessibilityLabel("Window end, 24-hour HH:mm")
                    }
                    Toggle("Suspend downloads and seeding",isOn:$window.suspend)
                    if !window.suspend {
                        HStack {
                            Text("Download"); TextField("0",text:$window.down).accessibilityLabel("Scheduled download limit in KB/s")
                            Text("Upload"); TextField("0",text:$window.up).accessibilityLabel("Scheduled upload limit in KB/s")
                            Text("KB/s")
                        }
                        Text("0 means unlimited. These caps replace normal limits during this window.").font(.system(size:10)).foregroundStyle(.secondary)
                    }
                }.font(.system(size:11)).textFieldStyle(.roundedBorder).padding(12).background(Design.canvas,in:RoundedRectangle(cornerRadius:8))
            }
            Text("Suspension preserves manual pause choices, file selections and queue order. Verification and file moves can finish.").font(.system(size:10)).foregroundStyle(.secondary)
            HStack {
                Button("Add window") { windows.append(ScheduleDraft(ScheduleRule())) }.disabled(windows.count >= 32)
                Spacer()
                Button("Reload saved",action:reload)
                Button(store.savingSchedule ? "Saving…" : "Save schedule",action:save)
            }
            if let message { Text(message).font(.system(size:11)).foregroundStyle(.secondary).announceAccessibility(message) }
        }
        .disabled(!store.canChangeTransfers || store.schedule == nil || store.savingSchedule)
        .onAppear(perform:reload)
        .onChange(of:store.schedule) { old, value in
            if old == nil, value != nil { reload() }
        }
    }
    private func reload() {
        let config = store.schedule ?? ScheduleConfig()
        enabled = config.enabled; windows = config.rules.map(ScheduleDraft.init); message = nil
    }
    private func save() {
        do {
            let value = ScheduleConfig(enabled:enabled,rules:try windows.map { try $0.rule() })
            Task {
                do { try await store.saveSchedule(value); message = "Schedule saved." }
                catch { message = error.localizedDescription }
            }
        } catch { message = error.localizedDescription }
    }
}

struct ScheduleDraft: Identifiable {
    let id = UUID()
    var days: UInt8
    var start: String
    var end: String
    var down: String
    var up: String
    var suspend: Bool
    init(_ rule: ScheduleRule) {
        days = rule.days; start = Self.time(rule.startMinute); end = Self.time(rule.endMinute)
        down = String(rule.downloadLimit / 1000); up = String(rule.uploadLimit / 1000); suspend = rule.suspend
    }
    static func time(_ minute: UInt16) -> String { String(format:"%02d:%02d",Int(minute / 60),Int(minute % 60)) }
    static func minute(_ text: String) -> UInt16? {
        let parts = text.split(separator:":",omittingEmptySubsequences:false)
        guard parts.count == 2, parts[0].count == 2, parts[1].count == 2,
              parts.allSatisfy({$0.utf8.allSatisfy({$0 >= 48 && $0 <= 57})}),
              let hour = UInt16(parts[0]), hour < 24, let minute = UInt16(parts[1]), minute < 60 else { return nil }
        return hour * 60 + minute
    }
    func rule() throws -> ScheduleRule {
        guard days > 0, let start = Self.minute(start), let end = Self.minute(end) else {
            throw UserFacingError(message:"Choose at least one day for each window and enter times as HH:mm, from 00:00 to 23:59.")
        }
        guard suspend || (UInt32(down).map({$0 <= UInt32.max / 1000}) == true && UInt32(up).map({$0 <= UInt32.max / 1000}) == true) else {
            throw UserFacingError(message:"Scheduled limits must be whole numbers from 0 to 4,294,967 KB/s.")
        }
        return ScheduleRule(days:days,startMinute:start,endMinute:end,downloadLimit:suspend ? 0 : UInt32(down)! * 1000,uploadLimit:suspend ? 0 : UInt32(up)! * 1000,suspend:suspend)
    }
}
