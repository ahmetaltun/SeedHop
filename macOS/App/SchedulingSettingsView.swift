import SwiftUI

/// Draft text remains local until the engine validates and durably saves it.
struct SchedulingSettingsView: View {
    @EnvironmentObject var store: LibraryStore
    @State private var enabled = false
    @State private var windows: [ScheduleDraft] = []
    @State private var message: String?
    static func orderedDays(calendar: Calendar = .current) -> [Int] {
        let mondayIndex = (calendar.firstWeekday + 5) % 7
        return (0..<7).map { (mondayIndex + $0) % 7 }
    }
    private func dayName(_ day: Int, abbreviated: Bool) -> String {
        let formatter = DateFormatter()
        formatter.locale = Locale(identifier:AppLocalization.shared.language)
        let symbols = abbreviated ? formatter.shortStandaloneWeekdaySymbols! : formatter.standaloneWeekdaySymbols!
        return symbols[(day + 1) % 7]
    }

    var body: some View {
        VStack(alignment:.leading,spacing:14) {
            Toggle(String(localized:"Enable weekly schedule",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale),isOn:$enabled).toggleStyle(.switch).controlSize(.small)
            Text(String(localized:"Windows use this Mac’s local time. Outside them, global bandwidth limits and queue rules apply. Alternative limits override scheduled caps; scheduled pauses still apply. Scheduling needs a running engine; it does not launch or wake the Mac.",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)).font(.system(size:11)).foregroundStyle(.secondary)
            if let warning = store.schedulingWarning { Text(warning).font(.system(size:11)).foregroundStyle(.orange) }
            Text(store.scheduleActive ? String(localized:"A schedule window is active.",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale) : String(localized:"Normal transfer settings are active.",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)).font(.system(size:11,weight:.medium))
            Text(String(localized:"Overnight windows end the following day. Equal start and end times mean 24 hours. Windows cannot overlap.",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)).font(.system(size:10)).foregroundStyle(.secondary)
            ForEach($windows) { $window in
                VStack(alignment:.leading,spacing:10) {
                    HStack {
                        Text(String(localized:"Window \((windows.firstIndex(where:{$0.id == window.id}) ?? 0) + 1)",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)).font(.system(size:12,weight:.semibold))
                        Spacer()
                        Button(String(localized:"Remove",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)) { let id = window.id; windows.removeAll { $0.id == id } }.accessibilityLabel(String(localized:"Remove schedule window \((windows.firstIndex(where:{$0.id == window.id}) ?? 0) + 1)",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale))
                    }
                    HStack(spacing:5) {
                        ForEach(Self.orderedDays(),id:\.self) { day in
                            Toggle(dayName(day,abbreviated:true),isOn:Binding(get:{window.days & (1 << day) != 0},set:{ selected in
                                if selected { window.days |= 1 << day } else { window.days &= ~(1 << day) }
                            })).toggleStyle(.button).accessibilityLabel(dayName(day,abbreviated:false))
                        }
                    }
                    HStack {
                        DatePicker(String(localized:"From",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale),selection:$window.startDate,displayedComponents:.hourAndMinute)
                            .accessibilityLabel(String(localized:"Schedule window start",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale))
                        DatePicker(String(localized:"To",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale),selection:$window.endDate,displayedComponents:.hourAndMinute)
                            .accessibilityLabel(String(localized:"Schedule window end",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale))
                    }
                    .environment(\.calendar,ScheduleDraft.timeCalendar)
                    .environment(\.timeZone,ScheduleDraft.timeCalendar.timeZone)
                    Toggle(String(localized:"Suspend downloads and seeding",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale),isOn:$window.suspend)
                    if !window.suspend {
                        HStack {
                            Text(String(localized:"Download",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)); TextField("0",text:$window.down).accessibilityLabel(String(localized:"Scheduled download limit in KB/s",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale))
                            Text(String(localized:"Upload",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)); TextField("0",text:$window.up).accessibilityLabel(String(localized:"Scheduled upload limit in KB/s",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale))
                            Text(String(localized:"KB/s",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale))
                        }
                        Text(String(localized:"0 means unlimited. These caps replace normal limits during this window.",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)).font(.system(size:10)).foregroundStyle(.secondary)
                    }
                }.font(.system(size:11)).textFieldStyle(.roundedBorder).padding(12).background(Design.canvas,in:RoundedRectangle(cornerRadius:8))
            }
            Text(String(localized:"Suspension preserves manual pause choices, file selections and queue order. Verification and file moves can finish.",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)).font(.system(size:10)).foregroundStyle(.secondary)
            HStack {
                Button(String(localized:"Add window",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)) { windows.append(ScheduleDraft(ScheduleRule())) }.disabled(windows.count >= 32)
                Spacer()
                Button(String(localized:"Reload saved",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale),action:reload)
                Button(store.savingSchedule ? String(localized:"Saving…",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale) : String(localized:"Save schedule",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale),action:save)
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
                do { try await store.saveSchedule(value); message = String(localized:"Schedule saved.",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale) }
                catch { message = error.localizedDescription }
            }
        } catch { message = error.localizedDescription }
    }
}

struct ScheduleDraft: Identifiable {
    // A time-only picker needs a date carrier. Fixed UTC prevents DST gaps from normalizing saved wall times.
    static var timeCalendar: Calendar {
        var calendar = Calendar(identifier:.gregorian)
        calendar.timeZone = TimeZone(secondsFromGMT:0)!
        return calendar
    }
    let id = UUID()
    var days: UInt8
    var start: String
    var end: String
    var down: String
    var up: String
    var suspend: Bool
    init(_ rule: ScheduleRule) {
        days = rule.days; start = Self.time(rule.startMinute); end = Self.time(rule.endMinute)
        down = LocaleText.integer(rule.downloadLimit / 1000); up = LocaleText.integer(rule.uploadLimit / 1000); suspend = rule.suspend
    }
    var startDate: Date {
        get { Self.date(Self.minute(start) ?? 0) }
        set { start = Self.time(Self.minute(newValue)) }
    }
    var endDate: Date {
        get { Self.date(Self.minute(end) ?? 0) }
        set { end = Self.time(Self.minute(newValue)) }
    }
    private static func date(_ minute: UInt16) -> Date {
        timeCalendar.date(from:DateComponents(year:2001,month:1,day:1,hour:Int(minute / 60),minute:Int(minute % 60)))!
    }
    private static func minute(_ date: Date) -> UInt16 {
        UInt16(timeCalendar.component(.hour,from:date) * 60 + timeCalendar.component(.minute,from:date))
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
            throw UserFacingError(message:String(localized:"Choose at least one day for each window and enter times as HH:mm, from 00:00 to 23:59.",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale))
        }
        guard suspend || (LocaleText.wholeNumber(down) != nil && LocaleText.wholeNumber(up) != nil) else {
            throw UserFacingError(message:String(localized:"Scheduled limits must be whole numbers from 0 to 4,294,967 KB/s.",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale))
        }
        return ScheduleRule(days:days,startMinute:start,endMinute:end,downloadLimit:suspend ? 0 : LocaleText.wholeNumber(down)! * 1000,uploadLimit:suspend ? 0 : LocaleText.wholeNumber(up)! * 1000,suspend:suspend)
    }
}
