import SwiftUI
import UniformTypeIdentifiers

struct ImportView: View {
    @EnvironmentObject var store: LibraryStore
    @Environment(\.dismiss) var dismiss
    @State var preview: ImportPreview?
    @State private var magnet = ""
    @State private var selected = Set<UInt32>()
    @State private var destination = ""
    @State private var resolving = false
    @State private var adding = false
    @State private var paused = false
    @State private var reuse = false
    @State private var error: String?
    @State private var requestId = UUID().uuidString
    @FocusState private var linkFocused: Bool

    init(preview: ImportPreview? = nil) {
        _preview = State(initialValue:preview)
        _selected = State(initialValue:Set(preview?.files.map(\.index) ?? []))
        _destination = State(initialValue:UserDefaults.standard.string(forKey:"downloadDirectory") ?? FileManager.default.urls(for:.downloadsDirectory,in:.userDomainMask)[0].path)
    }

    var body: some View {
        VStack(alignment:.leading,spacing:0) {
            HStack(alignment:.top,spacing:13) {
                BrandMark(size:42)
                VStack(alignment:.leading,spacing:5) {
                    Text(preview == nil ? "Make room for something good." : "Just the files you want.").font(.system(size:22,weight:.semibold,design:.rounded)).tracking(-0.4)
                    Text(preview == nil ? "Add a magnet link or open a torrent file." : "Choose what to download and where it belongs.").font(.system(size:12)).foregroundStyle(.secondary)
                }
                Spacer()
            }.padding(28)
            Divider()
            VStack(alignment:.leading,spacing:20) {
                if let preview {
                    HStack(spacing:12) {
                        Image(systemName:Design.symbol(preview.name)).font(.system(size:23,weight:.light)).foregroundStyle(Design.green).frame(width:44,height:48).background(Design.green.opacity(0.08),in:RoundedRectangle(cornerRadius:9))
                        VStack(alignment:.leading,spacing:5) { Text(preview.name).font(.system(size:14,weight:.semibold)).lineLimit(2); Text("\(preview.files.count) files · \(SizeText.bytes(preview.totalBytes))").font(.system(size:11)).foregroundStyle(.secondary) }
                    }
                    VStack(spacing:0) {
                        HStack { SectionEyebrow(title:"Files"); Spacer(); Button(selected.count == preview.files.count ? "Deselect all" : "Select all") { selected = selected.count == preview.files.count ? [] : Set(preview.files.map(\.index)) }.buttonStyle(.plain).font(.system(size:10)).foregroundStyle(Design.green) }.padding(13)
                        Divider()
                        ScrollView {
                            LazyVStack(spacing:0) {
                                ForEach(preview.files) { file in
                                    Toggle(isOn:Binding(get:{ selected.contains(file.index) },set:{ if $0 { selected.insert(file.index) } else { selected.remove(file.index) } })) {
                                        HStack(spacing:9) {
                                            Image(systemName:"doc").foregroundStyle(.secondary)
                                            Text(file.name).lineLimit(2).frame(maxWidth:.infinity,alignment:.leading)
                                            Text(SizeText.bytes(file.size)).foregroundStyle(.secondary).monospacedDigit()
                                        }.font(.system(size:11))
                                    }.toggleStyle(.checkbox).padding(.horizontal,13).padding(.vertical,11)
                                    if file.index != preview.files.last?.index { Divider().padding(.leading,34) }
                                }
                            }
                        }.frame(maxHeight:190)
                    }.background(Design.card,in:RoundedRectangle(cornerRadius:10)).clipShape(RoundedRectangle(cornerRadius:10)).overlay(RoundedRectangle(cornerRadius:10).stroke(Design.line))
                    VStack(alignment:.leading,spacing:11) {
                        SectionEyebrow(title:"Save to")
                        HStack(spacing:10) {
                            Image(systemName:"folder").foregroundStyle(Design.green)
                            Text(destination.replacingOccurrences(of:NSHomeDirectory(),with:"~")).font(.system(size:11)).lineLimit(1).truncationMode(.middle)
                            Spacer()
                            Button("Choose…",action:chooseFolder).controlSize(.small)
                        }.padding(12).background(Design.card,in:RoundedRectangle(cornerRadius:8)).overlay(RoundedRectangle(cornerRadius:8).stroke(Design.line))
                    }
                    VStack(alignment:.leading,spacing:9) {
                        Toggle("Add paused",isOn:$paused)
                        Toggle("Verify and reuse existing files",isOn:$reuse)
                    }.toggleStyle(.checkbox).font(.system(size:11))
                } else {
                    VStack(alignment:.leading,spacing:10) {
                        SectionEyebrow(title:"Magnet link")
                        TextField("magnet:?xt=urn:btih:…",text:$magnet).textFieldStyle(.plain).font(.system(size:12,design:.monospaced)).padding(13).background(Design.card,in:RoundedRectangle(cornerRadius:9)).overlay(RoundedRectangle(cornerRadius:9).stroke(linkFocused ? Design.green.opacity(0.6) : Design.line)).focused($linkFocused).onSubmit { resolve() }.disabled(resolving)
                        Text("A magnet link finds the torrent’s files through its peers.").font(.system(size:10)).foregroundStyle(.secondary)
                    }
                    HStack(spacing:13) { Rectangle().fill(Design.line).frame(height:1); Text("or").font(.system(size:11)).foregroundStyle(.tertiary); Rectangle().fill(Design.line).frame(height:1) }
                    Button(action:chooseFile) {
                        HStack(spacing:12) { Image(systemName:"doc.badge.plus").font(.system(size:23,weight:.light)).foregroundStyle(Design.green); VStack(alignment:.leading,spacing:4) { Text("Open a torrent file").font(.system(size:12,weight:.medium)); Text("Choose a .torrent file from your Mac").font(.system(size:10)).foregroundStyle(.secondary) }; Spacer(); Image(systemName:"chevron.right").font(.system(size:10)).foregroundStyle(.secondary) }.padding(17).contentShape(Rectangle())
                    }.buttonStyle(.plain).background(Design.green.opacity(0.035),in:RoundedRectangle(cornerRadius:10)).overlay(RoundedRectangle(cornerRadius:10).stroke(Design.green.opacity(0.13),style:StrokeStyle(lineWidth:1,dash:[4,3]))).disabled(resolving)
                    if resolving { HStack(spacing:10) { ProgressView().controlSize(.small); VStack(alignment:.leading,spacing:3) { Text("Finding the file list…").font(.system(size:12,weight:.medium)); Text("This may take a moment when peers are scarce.").font(.system(size:10)).foregroundStyle(.secondary) } }.padding(.vertical,4).accessibilityElement(children:.combine) }
                }
                if let error { InlineNotice(title:"Couldn’t add this torrent",message:error) }
            }.padding(28)
            Divider()
            HStack {
                if let preview { Text("\(selected.count) selected · \(SizeText.bytes(preview.files.filter { selected.contains($0.index) }.reduce(0) { $0 + $1.size }))").font(.system(size:10)).foregroundStyle(.secondary) }
                else { Label(store.designPreview ? "Design preview · sample data" : "BitTorrent v1 preview",systemImage:store.designPreview ? "paintpalette" : "info.circle").font(.system(size:10)).foregroundStyle(.secondary) }
                Spacer()
                Button("Cancel") { cancel(); dismiss() }.keyboardShortcut(.cancelAction)
                Button { preview == nil ? resolve() : add() } label: { HStack(spacing:6) { if adding { ProgressView().controlSize(.mini) }; Text(preview == nil ? "Continue" : paused ? "Add to Library" : "Start Download") } }
                    .buttonStyle(.borderedProminent).tint(Design.green).keyboardShortcut(.defaultAction)
                    .disabled(resolving || adding || (preview == nil ? magnet.trimmingCharacters(in:.whitespacesAndNewlines).isEmpty : selected.isEmpty))
            }.controlSize(.large).padding(20)
        }.frame(width:600).background(Design.canvas).tint(Design.green)
        .task {
            destination = store.downloadDirectory
            magnet = store.incomingMagnet
            if let preview { selected = Set(preview.files.map(\.index)) }
            else if let file = store.incomingFile { await inspect(file:file) }
            else { linkFocused = true }
        }
        .onDisappear { cancel() }
    }
    private func resolve() {
        guard !resolving else { return }
        resolving = true; error = nil
        Task {
            defer { resolving = false }
            do { var request = EngineRequest(command:"inspect"); request.requestId = requestId; request.magnet = magnet.trimmingCharacters(in:.whitespacesAndNewlines); accept(try await store.perform(request).preview) }
            catch { self.error = error.localizedDescription }
        }
    }
    private func inspect(file: URL) async {
        resolving = true; error = nil; defer { resolving = false }
        do {
            let size = try file.resourceValues(forKeys:[.fileSizeKey]).fileSize ?? 0
            guard size <= 32 * 1024 * 1024 else { throw UserFacingError(message:"The torrent metadata exceeds 32 MB. Choose a smaller .torrent file.") }
            var request = EngineRequest(command:"inspect"); request.requestId = requestId; request.bytes = try Data(contentsOf:file,options:.mappedIfSafe); accept(try await store.perform(request).preview)
        }
        catch { self.error = error.localizedDescription }
    }
    private func accept(_ value: ImportPreview?) { preview = value; selected = Set(value?.files.map(\.index) ?? []) }
    private func add() {
        guard let preview else { return }; adding = true; error = nil
        Task { defer { adding = false }; do { var r = EngineRequest(command:"add"); r.bytes = preview.torrentBytes; r.selected = Array(selected).sorted(); r.destination = destination; r.paused = paused; r.reuse = reuse; let response = try await store.perform(r); await store.refresh(); if let id = response.id { store.selection = [id] }; dismiss() } catch { self.error = error.localizedDescription } }
    }
    private func cancel() { guard resolving else { return }; var r = EngineRequest(command:"cancel"); r.id = requestId; Task { _ = try? await store.perform(r) } }
    private func chooseFolder() { let panel = NSOpenPanel(); panel.canChooseDirectories = true; panel.canChooseFiles = false; panel.directoryURL = URL(fileURLWithPath:destination); panel.begin { if $0 == .OK, let url = panel.url { destination = url.path } } }
    private func chooseFile() { let panel = NSOpenPanel(); panel.allowedContentTypes = [UTType(filenameExtension:"torrent")!]; panel.begin { if $0 == .OK, let url = panel.url { Task { await inspect(file:url) } } } }
}
