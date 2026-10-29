import SwiftUI
import UniformTypeIdentifiers

struct ImportView: View {
    @EnvironmentObject var store: LibraryStore
    @Environment(\.dismiss) var dismiss
    @StateObject private var session: ImportSession
    private enum FocusTarget: Hashable { case link, destination, sourceFile }
    @FocusState private var focus: FocusTarget?
    @FocusState private var filesFocused: Bool
    @State private var focusedFiles = Set<UInt32>()

    init(preview: ImportPreview? = nil) {
        _session = StateObject(wrappedValue:ImportSession(preview:preview))
    }

    var body: some View {
        VStack(alignment:.leading,spacing:0) {
            HStack(alignment:.top,spacing:13) {
                BrandMark(size:42)
                VStack(alignment:.leading,spacing:5) {
                    Text(session.preview == nil ? "Make room for something good." : "Just the files you want.").font(.system(size:22,weight:.semibold,design:.rounded)).tracking(-0.4)
                    Text(session.preview == nil ? "Add a magnet link or open a torrent file." : "Choose what to download and where it belongs.").font(.system(size:12)).foregroundStyle(.secondary)
                }
                Spacer()
            }.padding(28)
            Divider()
            VStack(alignment:.leading,spacing:20) {
                if let preview = session.preview {
                    HStack(spacing:12) {
                        Image(systemName:Design.symbol(preview.name)).font(.system(size:23,weight:.light)).foregroundStyle(Design.green).frame(width:44,height:48).background(Design.green.opacity(0.08),in:RoundedRectangle(cornerRadius:9))
                        VStack(alignment:.leading,spacing:5) { Text(preview.name).font(.system(size:14,weight:.semibold)).lineLimit(2); Text("\(preview.files.count) files · \(SizeText.bytes(preview.totalBytes))").font(.system(size:11)).foregroundStyle(.secondary) }
                    }
                    VStack(spacing:0) {
                        HStack { SectionEyebrow(title:"Files"); Spacer(); Button(session.selected.count == preview.files.count ? "Deselect all" : "Select all") { session.selected = session.selected.count == preview.files.count ? [] : Set(preview.files.map(\.index)) }.buttonStyle(.borderless).font(.system(size:10)).foregroundStyle(Design.green) }.padding(13)
                        Divider()
                        Table(preview.files,selection:$focusedFiles) {
                            TableColumn("File") { file in
                                Toggle(isOn:Binding(get:{ session.selected.contains(file.index) },set:{ if $0 { session.selected.insert(file.index) } else { session.selected.remove(file.index) } })) {
                                    Label(file.name,systemImage:"doc").font(.system(size:11)).lineLimit(2)
                                }.toggleStyle(.checkbox).accessibilityLabel("Download \(file.name)").accessibilityHint("\(SizeText.bytes(file.size))")
                            }.width(min:240,ideal:380)
                            TableColumn("Size") { file in Text(SizeText.bytes(file.size)).font(.system(size:11)).foregroundStyle(.secondary).monospacedDigit() }.width(75)
                        }.tableStyle(.inset(alternatesRowBackgrounds:false)).frame(height:190).focused($filesFocused)
                            .accessibilityLabel("Files to download")
                            .accessibilityHint("Use arrow keys to choose rows. Space toggles whether those files will download.")
                            .onKeyPress(.space) {
                                guard !focusedFiles.isEmpty, !session.adding else { return .ignored }
                                session.toggleFiles(focusedFiles); return .handled
                            }
                            .task { if let first = preview.files.first { focusedFiles = [first.index] }; focus = nil; await Task.yield(); filesFocused = true }

                    }.background(Design.card,in:RoundedRectangle(cornerRadius:10)).clipShape(RoundedRectangle(cornerRadius:10)).overlay(RoundedRectangle(cornerRadius:10).stroke(Design.line))
                    VStack(alignment:.leading,spacing:11) {
                        SectionEyebrow(title:"Save to")
                        HStack(spacing:10) {
                            Image(systemName:"folder").foregroundStyle(Design.green)
                            Text(session.destination.replacingOccurrences(of:NSHomeDirectory(),with:"~")).font(.system(size:11)).lineLimit(1).truncationMode(.middle)
                            Spacer()
                            Button("Choose…",action:chooseFolder).controlSize(.small).focused($focus,equals:.destination).accessibilityLabel("Choose download folder")
                        }.padding(12).background(Design.card,in:RoundedRectangle(cornerRadius:8)).overlay(RoundedRectangle(cornerRadius:8).stroke(Design.line))
                    }
                    HStack(alignment:.top,spacing:24) {
                        VStack(alignment:.leading,spacing:9) {
                            Toggle("Add to top of queue",isOn:$session.addToTop)
                                .help("Start before other waiting downloads. Active downloads keep their slots.")
                            Toggle("Download in sequential order",isOn:$session.sequential)
                                .help("Prefer earlier pieces in each selected file. Otherwise, prefer rarer available pieces.")
                            Toggle("Download first and last pieces first",isOn:$session.firstLastPieces)
                                .help("Prioritize the first and last pieces of each selected file, then use the chosen download order.")
                        }.frame(maxWidth:.infinity,alignment:.leading)
                        VStack(alignment:.leading,spacing:9) {
                            Toggle("Add paused",isOn:$session.paused)
                            Toggle("Verify and reuse existing files",isOn:$session.reuse)
                        }.frame(maxWidth:.infinity,alignment:.leading)
                    }.toggleStyle(.checkbox).font(.system(size:11))
                } else {
                    VStack(alignment:.leading,spacing:10) {
                        SectionEyebrow(title:"Magnet link")
                        TextField("magnet:?xt=urn:btih:…",text:$session.magnet).textFieldStyle(.plain).font(.system(size:12,design:.monospaced)).padding(13).background(Design.card,in:RoundedRectangle(cornerRadius:9)).overlay(RoundedRectangle(cornerRadius:9).stroke(focus == .link ? Design.green : Design.line, lineWidth:focus == .link ? 2 : 1)).focused($focus,equals:.link).onSubmit { resolve() }.disabled(session.resolving).accessibilityLabel("Magnet link")
                        Text("A magnet link finds the torrent’s files through its peers.").font(.system(size:10)).foregroundStyle(.secondary)
                    }
                    HStack(spacing:13) { Rectangle().fill(Design.line).frame(height:1); Text("or").font(.system(size:11)).foregroundStyle(.tertiary); Rectangle().fill(Design.line).frame(height:1) }
                    Button(action:chooseFile) {
                        HStack(spacing:12) { Image(systemName:"doc.badge.plus").font(.system(size:23,weight:.light)).foregroundStyle(Design.green); VStack(alignment:.leading,spacing:4) { Text("Open a torrent file").font(.system(size:12,weight:.medium)); Text("Choose a .torrent file from your Mac").font(.system(size:10)).foregroundStyle(.secondary) }; Spacer(); Image(systemName:"chevron.right").font(.system(size:10)).foregroundStyle(.secondary) }.padding(17).contentShape(Rectangle())
                    }.buttonStyle(.borderless).background(Design.green.opacity(0.035),in:RoundedRectangle(cornerRadius:10)).overlay(RoundedRectangle(cornerRadius:10).stroke(Design.green.opacity(0.13),style:StrokeStyle(lineWidth:1,dash:[4,3]))).disabled(session.resolving).focused($focus,equals:.sourceFile).accessibilityLabel("Open a torrent file")
                    if let file = session.sourceFile {
                        Label(file.lastPathComponent,systemImage:"doc").font(.system(size:11)).foregroundStyle(.secondary).lineLimit(1).truncationMode(.middle).help(file.path)
                    }
                    if session.resolving { HStack(spacing:10) { ProgressView().controlSize(.small); VStack(alignment:.leading,spacing:3) { Text("Finding the file list…").font(.system(size:12,weight:.medium)); Text(session.sourceFile == nil ? "This may take a moment when peers are scarce." : "Reading your torrent file…").font(.system(size:10)).foregroundStyle(.secondary) } }.padding(.vertical,4).accessibilityElement(children:.combine) }
                }
                if let error = session.error {
                    InlineNotice(title:"Couldn’t add this torrent",message:error,retry:session.sourceFile != nil && session.preview == nil && !session.resolving ? {
                        if let file = session.sourceFile { Task { await session.inspect(file:file,using:store.perform) } }
                    } : nil)
                }
            }.padding(28).disabled(session.adding)
            Divider()
            HStack {
                if let preview = session.preview { Text("\(session.selected.count) selected · \(SizeText.bytes(preview.files.filter { session.selected.contains($0.index) }.reduce(0) { $0 + $1.size }))").font(.system(size:10)).foregroundStyle(.secondary) }
                else { Label(store.designPreview ? "Design preview · sample data" : "BitTorrent v1 + v2",systemImage:store.designPreview ? "paintpalette" : "info.circle").font(.system(size:10)).foregroundStyle(.secondary) }
                Spacer()
                Button("Cancel") { cancel(); dismiss() }.keyboardShortcut(.cancelAction).disabled(session.adding)
                Button { session.preview == nil ? resolve() : add() } label: { HStack(spacing:6) { if session.adding { ProgressView().controlSize(.mini) }; Text(session.adding ? "Adding…" : session.preview == nil ? "Continue" : session.paused ? "Add to Library" : "Start Download") } }
                    .buttonStyle(.borderedProminent).tint(Design.green).keyboardShortcut(.defaultAction)
                    .disabled(store.hasFileOperation || session.resolving || session.adding || (session.preview == nil ? session.magnet.trimmingCharacters(in:.whitespacesAndNewlines).isEmpty : session.selected.isEmpty))
            }.controlSize(.large).padding(20)
        }.frame(width:600).background(Design.canvas).tint(Design.green)
        .interactiveDismissDisabled(session.adding)
        .task {
            session.destination = store.downloadDirectory
            session.magnet = store.incomingMagnet
            if session.preview == nil, let file = store.incomingFile { await session.inspect(file:file,using:store.perform) }
            else if session.preview == nil { focus = .link }
        }
        .onChange(of:session.error) { _, error in
            if error != nil && session.preview == nil { focus = session.sourceFile == nil ? .link : .sourceFile }
        }
        .onDisappear { cancel() }
    }
    private func resolve() { Task { await session.resolve(using:store.perform) } }
    private func add() {
        Task {
            if let id = await session.add(using:store.perform) {
                await store.refresh()
                store.revealTransfer(id)
                dismiss()
            }
        }
    }
    private func cancel() { session.cancel(using:store.perform) }
    private func chooseFolder() {
        let panel = NSOpenPanel(); panel.canChooseDirectories = true; panel.canChooseFiles = false
        panel.directoryURL = URL(fileURLWithPath:session.destination)
        panel.begin { if $0 == .OK, let url = panel.url { session.destination = url.path }; focus = .destination }
    }
    private func chooseFile() {
        let panel = NSOpenPanel(); panel.allowedContentTypes = [UTType(filenameExtension:"torrent")!]
        panel.begin { focus = .sourceFile; if $0 == .OK, let url = panel.url { Task { await session.inspect(file:url,using:store.perform) } } }
    }
}
