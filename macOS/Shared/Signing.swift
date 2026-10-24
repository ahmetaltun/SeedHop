import Foundation
import Security

enum Signing {
    static func requirement(at url: URL) throws -> String {
        var code: SecStaticCode?
        guard SecStaticCodeCreateWithPath(url as CFURL, [], &code) == errSecSuccess, let code else { throw UserFacingError(message: "The bundled helper signature could not be read.") }
        var requirement: SecRequirement?
        guard SecCodeCopyDesignatedRequirement(code, [], &requirement) == errSecSuccess, let requirement else { throw UserFacingError(message: "The app must be signed before connecting to its engine.") }
        var text: CFString?
        guard SecRequirementCopyString(requirement, [], &text) == errSecSuccess, let text else { throw UserFacingError(message: "The signing requirement could not be read.") }
        return text as String
    }
    static var containingApp: URL {
        var url = Bundle.main.bundleURL
        while url.path != "/" {
            if url.pathExtension == "app" { return url }
            url.deleteLastPathComponent()
        }
        return Bundle.main.bundleURL
    }
}
