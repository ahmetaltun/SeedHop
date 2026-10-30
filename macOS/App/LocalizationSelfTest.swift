import Foundation
import AppKit

#if SEEDHOP_DEVELOPMENT
@MainActor enum LocalizationSelfTest {
    static func runLive(store provider: () -> LibraryStore?) async throws {
        func descendants(_ view: NSView) -> [NSView] { [view] + view.subviews.flatMap(descendants) }
        try await Task.sleep(for:.milliseconds(400))
        try await InterfaceSelfTest.invoke("Show Library",key:"0")
        let deadline = Date(timeIntervalSinceNow:5)
        while Date() < deadline {
            if let window = NSApp.windows.first(where:{ $0.title == "SeedHop" }), let content = window.contentView,
               descendants(content).contains(where:{ $0 is NativeTransfersTable }), provider() != nil { break }
            try await Task.sleep(for:.milliseconds(100))
        }
        guard let store = provider(), let window = NSApp.windows.first(where:{ $0.title == "SeedHop" }),
              let content = window.contentView,
              let table = descendants(content).compactMap({ $0 as? NativeTransfersTable }).first else {
            throw UserFacingError(message:"Live language fixture has no native table")
        }
        let defaults = WorkspacePreferences.defaults
        let oldPreference = defaults.persistentDomain(forName:"app.seedhop.workspace-fixtures")?["AppleLanguages"]
        let originalLanguage = AppLocalization.shared.language
        defer {
            if let oldPreference { defaults.set(oldPreference,forKey:"AppleLanguages") }
            else { defaults.removeObject(forKey:"AppleLanguages") }
            AppLocalization.shared.use(originalLanguage)
        }
        let selection = store.selection, torrents = store.torrents
        let name = table.tableColumns.first(where:{ $0.identifier.rawValue == "name" })!
        name.width = 275
        for (language, title) in [("tr","Ad"),("en","Name"),("fr","Nom"),("ru","Имя")] {
            AppLanguagePreference.save(language)
            try await Task.sleep(for:.milliseconds(350))
            guard descendants(content).contains(where:{ $0 === table }), name.title == title,
                  abs(name.width - 275) < 0.5, store.selection == selection, store.torrents == torrents else {
                throw UserFacingError(message:"Live language did not refresh native labels or preserve table state: \(language), \(name.title)")
            }
            let menuTitle = String(localized:"Transfers",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale)
            guard NSApp.mainMenu?.items.contains(where:{ $0.title == menuTitle }) == true else {
                throw UserFacingError(message:"Live language did not update app command menu: \(language)")
            }
            let labels = NativeMenuLocalization()
            guard let main = NSApp.mainMenu, let appMenu = main.items.first?.submenu else {
                throw UserFacingError(message:"Live language fixture has no application menu")
            }
            appMenu.delegate?.menuNeedsUpdate?(appMenu); appMenu.update()
            try await Task.sleep(for:.milliseconds(100))
            let appLabels = ["About %@","Settings…","Services","Hide %@","Hide Others","Show All","Quit %@"]
            guard appLabels.allSatisfy({ key in appMenu.items.contains(where:{ $0.title == labels.title(for:key) }) }),
                  let quit = appMenu.items.first(where:{ $0.action == NSSelectorFromString("terminate:") }),
                  quit.keyEquivalent == "q", quit.keyEquivalentModifierMask == .command else {
                throw UserFacingError(message:"Standard application menu stayed in the launch language or lost shortcuts: \(language), \(appMenu.items.map(\.title))")
            }
            if language == "en" {
                guard main.items.map(\.title) == ["SeedHop","File","Edit","View","Transfers","Window","Help"],
                      appMenu.items.contains(where:{ $0.title == "About SeedHop" }),
                      appMenu.items.contains(where:{ $0.title == "Settings…" }),
                      appMenu.items.contains(where:{ $0.title == "Quit SeedHop" }) else {
                    throw UserFacingError(message:"Turkish launch menus did not become English")
                }
            }
        }
        let count = 21
        guard String(localized:"\(count) transfers",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale) == "21 передача",
              AppLocalization.shared.locale.region == Locale.current.region else {
            throw UserFacingError(message:"Live language lost plural rules or regional preferences")
        }
        print("PASS: Turkish, English, French, and Russian apply in one process, including standard app/menu-bar labels; shortcuts, table identity, widths, selection, transfer data, plural rules, and region survive.")
    }
    static func run() throws {
        var checks = 0
        func expect(_ value: @autoclosure () throws -> Bool, _ detail: String) throws {
            guard try value() else { throw UserFacingError(message:detail) }
            checks += 1
        }
        for (identifier, grouped) in [("en_US","1,234"),("de_DE","1.234"),("fr_FR","1\u{202f}234"),("tr_TR","1.234"),("pl_PL","1\u{00a0}234")] {
            let locale = Locale(identifier:identifier)
            try expect(LocaleText.wholeNumber(grouped,locale:locale) == 1234,"Regional grouped number failed: \(identifier)")
            try expect(LocaleText.wholeNumber("1234",locale:locale) == 1234,"Plain integer failed")
            for invalid in ["-1","+1","1.5","1,5","12x","12 34","4294968","NaN",""] {
                try expect(LocaleText.wholeNumber(invalid,locale:locale) == nil,"Invalid number accepted: \(identifier), \(invalid)")
            }
            try expect(LocaleText.wholeNumber("4294967",locale:locale) == 4_294_967,"Maximum rejected")
            try expect(LocaleText.wholeNumber("0",locale:locale) == 0,"Unlimited rejected")
        }
        var calendar = Calendar(identifier:.gregorian)
        calendar.firstWeekday = 1
        try expect(SchedulingSettingsView.orderedDays(calendar:calendar) == [6,0,1,2,3,4,5],"Sunday-first order changed day bits")
        calendar.firstWeekday = 2
        try expect(SchedulingSettingsView.orderedDays(calendar:calendar) == Array(0..<7),"Monday-first order changed day bits")
        try expect(LocaleText.percent(0.999).contains("99") && !LocaleText.percent(0.999).contains("100"),"Unfinished progress rounded up to complete")
        do {
            let languageDefaults = WorkspacePreferences.defaults
            let initialLocalization = AppLocalization.shared.language
            let oldLanguage = languageDefaults.persistentDomain(forName:"app.seedhop.workspace-fixtures")?["AppleLanguages"]
            defer {
                AppLocalization.shared.use(initialLocalization)
                if let oldLanguage { languageDefaults.set(oldLanguage,forKey:"AppleLanguages") }
                else { languageDefaults.removeObject(forKey:"AppleLanguages") }
            }
            try expect(AppLanguagePreference.available.count == 13,"Settings language list differs from bundled languages")
            AppLanguagePreference.save("fr")
            try expect(AppLanguagePreference.saved == "fr","Settings language selection was not persisted")
            AppLanguagePreference.save("")
            try expect(AppLanguagePreference.saved.isEmpty && languageDefaults.persistentDomain(forName:"app.seedhop.workspace-fixtures")?["AppleLanguages"] == nil,"Follow macOS retained an app language override")
            AppLanguagePreference.save("ar")
            try expect(AppLanguagePreference.saved.isEmpty,"An unsupported language was persisted")
        }
        var draft = ScheduleDraft(ScheduleRule(days:64,startMinute:1439,endMinute:0))
        let before = try draft.rule()
        draft.startDate = draft.startDate; draft.endDate = draft.endDate
        try expect(try draft.rule() == before,"Native time controls changed saved minute values")
        draft.start = "02:30"
        try expect(ScheduleDraft.timeCalendar.timeZone.secondsFromGMT() == 0 && ScheduleDraft.timeCalendar.component(.hour,from:draft.startDate) == 2,"Time-only carrier depends on DST")
        draft.startDate = draft.startDate
        try expect(draft.start == "02:30","DST-gap wall time changed")
        let message = EngineMessage(code:"future.unknown",arguments:[.text("résumé/文件.txt"),.number(21)],diagnostic:"raw tracker detail")
        let data = try JSONEncoder().encode(message)
        try expect(try JSONDecoder().decode(EngineMessage.self,from:data) == message,"Structured argument round trip failed")
        try expect(message.displayText.contains("raw tracker detail"),"Unknown code lost diagnostic")
        let reply = try JSONDecoder().decode(EngineReply.self,from:Data("{\"version\":6,\"error\":\"legacy detail\"}".utf8))
        try expect(reply.errorMessage == nil && reply.error == "legacy detail","Legacy reply cannot be decoded")
        let legacySnapshot = Data("{\"protocolVersion\":5,\"engineVersion\":\"legacy\",\"instanceId\":\"fixture\",\"listenPort\":0,\"downloadLimit\":0,\"uploadLimit\":0,\"maxActiveDownloads\":0,\"torrents\":[]}".utf8)
        let decoded = try JSONDecoder().decode(LibrarySnapshot.self,from:legacySnapshot)
        try expect(decoded.statisticsMessage == nil && decoded.schedulingMessage == nil && decoded.protocolVersion == 5,"Legacy snapshot requires new message fields")
        let preferences = WorkspacePreferences.defaults
        let previousWidths = preferences.object(forKey:"workspaceColumnWidths")
        defer {
            if let previousWidths { preferences.set(previousWidths,forKey:"workspaceColumnWidths") }
            else { preferences.removeObject(forKey:"workspaceColumnWidths") }
        }
        preferences.set(["Name":275.0],forKey:"workspaceColumnWidths")
        let table = NSTableView()
        let column = NSTableColumn(identifier:NSUserInterfaceItemIdentifier("name"))
        column.title = String(localized:"Name",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale); table.addTableColumn(column)
        WorkspaceTablePersistence.install(table)
        try expect(abs(column.width - 275) < 0.5,"English legacy heading width was not restored")
        column.width = 310
        NotificationCenter.default.post(name:NSTableView.columnDidResizeNotification,object:table)
        try expect((preferences.dictionary(forKey:"workspaceColumnWidths")?["name"] as? Double) == 310,"Column width was not saved under stable ID")
        preferences.set(["name":310.0],forKey:"workspaceColumnWidths")
        let translatedTable = NSTableView(), translatedColumn = NSTableColumn(identifier:NSUserInterfaceItemIdentifier("name"))
        translatedColumn.title = "名称"; translatedTable.addTableColumn(translatedColumn)
        WorkspaceTablePersistence.install(translatedTable)
        try expect(abs(translatedColumn.width - 310) < 0.5,"A heading language change lost saved width")
        let names = ["İstanbul.torrent","résumé.txt","文件.bin","ملف.txt"]
        for name in names {
            let file = FileItem(index:0,name:name,size:1,downloaded:0,selected:true)
            try expect(try JSONDecoder().decode(FileItem.self,from:JSONEncoder().encode(file)).name == name,"Unicode filename changed")
        }
        let counts = [0,1,2,5,11,21]
        let labels = counts.map { count in String(localized:"Remove \(count) transfers?",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale) }
        for (count,label) in zip(counts,labels) {
            try expect(!label.contains("%lld") && label.contains(String(count)),"Plural placeholder did not resolve")
        }
        var statusSamples: [String:[String]] = [:]
        let store = LibraryStore(designPreview:true)
        for state in ["checking","downloading","paused","queued","scheduled","seeding","waiting"] {
            statusSamples[state] = [1,2,5,11,21].map { count in
                store.torrents = (0..<count).map { index in
                    var torrent = PreviewFixtures.torrents[0]
                    torrent.id = "fixture-\(index)"; torrent.state = state
                    return torrent
                }
                return store.activitySummary
            }
        }
        let language = Bundle.main.preferredLocalizations.first ?? "en"
        let report: [String:Any] = ["checks":checks,"language":language,"locale":Locale.current.identifier,
            "remove":String(localized:"Remove",bundle:AppLocalization.shared.bundle,locale:AppLocalization.shared.locale),"counts":labels,"regionalNumber":1234.5.formatted(.number),"englishFallback":Bundle.main.developmentLocalization ?? "",
            "supported":Bundle.main.localizations.sorted(),"statusSamples":statusSamples]
        let output = try JSONSerialization.data(withJSONObject:report,options:[.prettyPrinted,.sortedKeys])
        print(String(decoding:output,as:UTF8.self))
    }
}
#endif
