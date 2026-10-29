import SwiftUI
import AppKit

/// AppKit owns divider persistence and minimum pane sizes. Hosted SwiftUI views
/// retain their structural identity when live transfer values change.
struct WorkspaceSplitView<Leading: View, Trailing: View>: NSViewRepresentable {
    let vertical: Bool
    let autosaveName: String
    let initialLeading: CGFloat
    let minLeading: CGFloat
    let minTrailing: CGFloat
    private let leading: Leading
    private let trailing: Trailing
    @Environment(\.colorScheme) private var colorScheme
    @Environment(\.accessibilityAnnouncementsEnabled) private var announcementsEnabled

    init(vertical: Bool, autosaveName: String, initialLeading: CGFloat, minLeading: CGFloat, minTrailing: CGFloat,
         @ViewBuilder leading: () -> Leading, @ViewBuilder trailing: () -> Trailing) {
        self.vertical = vertical; self.autosaveName = autosaveName
        self.initialLeading = initialLeading; self.minLeading = minLeading; self.minTrailing = minTrailing
        self.leading = leading(); self.trailing = trailing()
    }
    final class Coordinator: NSObject, NSSplitViewDelegate {
        let first: WorkspaceHostModel<Leading>
        let second: WorkspaceHostModel<Trailing>
        let key: String
        let initialLeading: CGFloat
        let minLeading: CGFloat
        let minTrailing: CGFloat
        private var position: CGFloat?
        private var lastExtent: CGFloat = 0
        private var resizing = false
        init(_ parent: WorkspaceSplitView) {
            first = WorkspaceHostModel(parent.leading); second = WorkspaceHostModel(parent.trailing)
            key = "workspaceDivider." + parent.autosaveName
            initialLeading = parent.initialLeading; minLeading = parent.minLeading; minTrailing = parent.minTrailing
        }
        private func clamp(_ proposed: CGFloat, in split: NSSplitView) -> CGFloat {
            let extent = split.isVertical ? split.bounds.width : split.bounds.height
            let available = max(0,extent-split.dividerThickness)
            let minimum = min(minLeading,available)
            return min(max(proposed,minimum),max(minimum,available-minTrailing))
        }
        func layout(_ split: NSSplitView) {
            guard !resizing, split.arrangedSubviews.count == 2 else { return }
            let extent = split.isVertical ? split.bounds.width : split.bounds.height
            guard extent > 0 else { return }
            var proposed: CGFloat
            if let position {
                proposed = split.isVertical ? position : position + extent-lastExtent
            } else {
                let saved = WorkspacePreferences.defaults.object(forKey:key) as? Double
                if split.isVertical { proposed = CGFloat(saved ?? Double(initialLeading)) }
                else {
                    let trailing = CGFloat(saved ?? Double(-initialLeading))
                    proposed = extent-trailing-split.dividerThickness
                }
            }
            let first = clamp(proposed,in:split), last = max(0,extent-first-split.dividerThickness)
            resizing = true; defer { resizing = false }
            let firstFrame: NSRect, lastFrame: NSRect
            if split.isVertical {
                firstFrame = NSRect(x:0,y:0,width:first,height:split.bounds.height)
                lastFrame = NSRect(x:first+split.dividerThickness,y:0,width:last,height:split.bounds.height)
            } else {
                firstFrame = NSRect(x:0,y:0,width:split.bounds.width,height:first)
                lastFrame = NSRect(x:0,y:first+split.dividerThickness,width:split.bounds.width,height:last)
            }
            if split.arrangedSubviews[0].frame != firstFrame { split.arrangedSubviews[0].frame = firstFrame }
            if split.arrangedSubviews[1].frame != lastFrame { split.arrangedSubviews[1].frame = lastFrame }
            position = first; lastExtent = extent
        }
        func splitView(_ split: NSSplitView, resizeSubviewsWithOldSize oldSize: NSSize) { layout(split) }
        func splitView(_ split: NSSplitView, constrainSplitPosition proposedPosition: CGFloat, ofSubviewAt dividerIndex: Int) -> CGFloat { clamp(proposedPosition,in:split) }
        func splitViewDidResizeSubviews(_ notification: Notification) {
            guard !resizing, position != nil, lastExtent > 0, let split = notification.object as? NSSplitView, split.arrangedSubviews.count == 2 else { return }
            let extent = split.isVertical ? split.bounds.width : split.bounds.height
            guard abs(extent-lastExtent) < 0.5 else { layout(split); return }
            let leading = split.isVertical ? split.arrangedSubviews[0].frame.width : split.arrangedSubviews[0].frame.height
            position = leading
            let saved = split.isVertical ? leading : split.arrangedSubviews[1].frame.height
            WorkspacePreferences.defaults.set(Double(saved),forKey:key)
        }
    }
    func makeCoordinator() -> Coordinator { Coordinator(self) }
    func sizeThatFits(_ proposal: ProposedViewSize, nsView: WorkspaceNativeSplitView, context: Context) -> CGSize? {
        // Panes fill their proposed workspace. Asking AppKit for a fitting size
        // recursively measures every hosted table and repeats constraint work.
        proposal.replacingUnspecifiedDimensions(by:CGSize(width:780,height:560))
    }
    func makeNSView(context: Context) -> WorkspaceNativeSplitView {
        let split = WorkspaceNativeSplitView()
        split.isVertical = vertical; split.dividerStyle = .thin; split.delegate = context.coordinator
        context.coordinator.first.colorScheme = colorScheme; context.coordinator.second.colorScheme = colorScheme
        context.coordinator.first.announcementsEnabled = announcementsEnabled; context.coordinator.second.announcementsEnabled = announcementsEnabled
        let first = NSHostingView(rootView:WorkspaceHostedContent(model:context.coordinator.first))
        let second = NSHostingView(rootView:WorkspaceHostedContent(model:context.coordinator.second))
        first.sizingOptions = []; second.sizingOptions = []
        split.addArrangedSubview(first); split.addArrangedSubview(second)
        split.didLayout = { [weak coordinator = context.coordinator] split in coordinator?.layout(split) }
        split.needsLayout = true
        return split
    }
    func updateNSView(_ split: WorkspaceNativeSplitView, context: Context) {
        context.coordinator.first.colorScheme = colorScheme; context.coordinator.second.colorScheme = colorScheme
        context.coordinator.first.announcementsEnabled = announcementsEnabled; context.coordinator.second.announcementsEnabled = announcementsEnabled
        context.coordinator.first.content = leading; context.coordinator.second.content = trailing
        // Content observes its own updates. Native frame and divider changes
        // already schedule split layout; transfer samples do not resize panes.
    }
}

/// Updating a model preserves the hosting root and descendant SwiftUI state.
final class WorkspaceHostModel<Content: View>: ObservableObject {
    @Published var content: Content
    var colorScheme: ColorScheme = .light
    var announcementsEnabled = true
    init(_ content: Content) { self.content = content }
}
private struct WorkspaceHostedContent<Content: View>: View {
    @ObservedObject var model: WorkspaceHostModel<Content>
    var body: some View { model.content.environment(\.colorScheme,model.colorScheme).environment(\.accessibilityAnnouncementsEnabled,model.announcementsEnabled) }
}
final class WorkspaceNativeSplitView: NSSplitView {
    var didLayout: ((NSSplitView) -> Void)?
    override var isFlipped: Bool { true }
    override func layout() { super.layout(); didLayout?(self) }
}

/// Native column autosave complements SwiftUI's persisted order and visibility.
@MainActor enum WorkspaceTablePersistence {
    private final class WeakTable {
        weak var value: NSTableView?
        var observer: NSObjectProtocol?
        var restoring = false
        init(_ value: NSTableView) { self.value = value }
        deinit { if let observer { NotificationCenter.default.removeObserver(observer) } }
    }
    private static var tables: [WeakTable] = []
    static func install(_ table: NSTableView) {
        tables.removeAll { $0.value == nil }
        if let entry = tables.first(where:{ $0.value === table }) {
            entry.restoring = true
            let saved = WorkspacePreferences.defaults.dictionary(forKey:"workspaceColumnWidths") as? [String:Double] ?? [:]
            for column in table.tableColumns { if let width = saved[column.headerCell.stringValue], abs(column.width-CGFloat(width)) > 0.5 { column.width = CGFloat(width) } }
            entry.restoring = false
            return
        }
        let entry = WeakTable(table); entry.restoring = true
        // SwiftUI gives native columns fresh UUIDs on each mount. Stable public
        // headings own width persistence; SwiftUI customization owns visibility/order.
        let saved = WorkspacePreferences.defaults.dictionary(forKey:"workspaceColumnWidths") as? [String:Double] ?? [:]
        for column in table.tableColumns { if let width = saved[column.headerCell.stringValue] { column.width = CGFloat(width) } }
        entry.restoring = false
        entry.observer = NotificationCenter.default.addObserver(forName:NSTableView.columnDidResizeNotification,object:nil,queue:.main) { [weak entry] notification in
            MainActor.assumeIsolated {
                guard let entry, !entry.restoring, let table = entry.value, (notification.object as? NSTableView) === table else { return }
                saveWidths(table)
            }
        }
        tables.append(entry)
    }
    private static func saveWidths(_ table: NSTableView) {
        var widths: [String:Double] = [:]
        for column in table.tableColumns where !column.headerCell.stringValue.isEmpty { widths[column.headerCell.stringValue] = Double(column.width) }
        WorkspacePreferences.defaults.set(widths,forKey:"workspaceColumnWidths")
    }
    static func restoreWidths() {
        let widths: [String:CGFloat] = ["Name":240,"Status":125,"Progress":90,"↓ Download":80,"↑ Upload":80,"Peers":50,"ETA":75,"Ratio":55,"Queue":55,"Selected Size":90,"Total Size":90,"Remaining":90,"Downloaded Traffic":115,"Uploaded Traffic":110,"Added":100,"Completed":100,"Save Path":220]
        for entry in tables { if let table = entry.value {
            entry.restoring = true
            for column in table.tableColumns { if let width = widths[column.headerCell.stringValue] { column.width = width } }
            entry.restoring = false; saveWidths(table)
        }}
    }
}

/// This marker belongs to the transfer table's background, so persistence never
/// attaches to an inspector table or an empty-state replacement.
@MainActor final class WorkspaceNativeSelection: ObservableObject {
    var synchronizing = false
}
struct NativeTransferTableMarker: NSViewRepresentable {
    let rowHeight: CGFloat
    let rowIDs: [String]
    let generation: UInt64
    let selection: IndexSet
    let selectionBridge: WorkspaceNativeSelection
    func makeNSView(context: Context) -> TransferTableMarkerView { let view = TransferTableMarkerView(); configure(view); return view }
    func updateNSView(_ view: TransferTableMarkerView, context: Context) { configure(view); view.schedule() }
    private func configure(_ view: TransferTableMarkerView) {
        view.rowHeight = rowHeight; view.rowCount = rowIDs.count; view.selection = selection; view.selectionBridge = selectionBridge
    }
}
final class TransferTableMarkerView: NSView {
    var rowHeight: CGFloat = 28
    var rowCount = 0
    var selection = IndexSet()
    weak var selectionBridge: WorkspaceNativeSelection?
    private var scheduled = false
    private var attempts = 0
    override var intrinsicContentSize: NSSize { NSSize(width:1,height:1) }
    override func viewDidMoveToSuperview() { super.viewDidMoveToSuperview(); schedule() }
    override func viewDidMoveToWindow() { super.viewDidMoveToWindow(); schedule() }
    func schedule() {
        guard !scheduled else { return }; scheduled = true
        DispatchQueue.main.asyncAfter(deadline:.now() + 0.01) { [weak self] in
            guard let self else { return }; self.scheduled = false
            func find(_ view: NSView) -> NSTableView? {
                if let table = view as? NSTableView, table.identifier?.rawValue != "torrent-files-outline", table.tableColumns.contains(where:{ $0.headerCell.stringValue == "Name" }) { return table }
                for child in view.subviews { if let table = find(child) { return table } }
                return nil
            }
            var ancestor = self.superview
            while let view = ancestor {
                if let table = find(view) {
                    // SwiftUI must measure hosted cells when a table is inserted
                    // into an empty library; disabling automatic heights hides text.
                    let height = max(1,self.rowHeight)
                    if table.intercellSpacing.height != 0 { table.intercellSpacing = NSSize(width:table.intercellSpacing.width,height:0) }
                    if table.rowSizeStyle != .custom { table.rowSizeStyle = .custom }
                    if table.columnAutoresizingStyle != .noColumnAutoresizing { table.columnAutoresizingStyle = .noColumnAutoresizing }
                    if table.rowHeight != height { table.rowHeight = height }
                    WorkspaceTablePersistence.install(table)
                    if table.numberOfRows == self.rowCount {
                        if table.selectedRowIndexes != self.selection {
                            // Model identity already owns selection. Suppress writeback
                            // while restoring its native highlight after row updates.
                            self.selectionBridge?.synchronizing = true
                            table.selectRowIndexes(self.selection,byExtendingSelection:false)
                            self.selectionBridge?.synchronizing = false
                        }
                        return
                    }
                }
                ancestor = view.superview
            }
            self.attempts += 1
            if self.attempts < 20 { self.schedule() }
        }
    }
}
