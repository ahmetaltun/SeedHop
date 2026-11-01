import Foundation
import CryptoKit

// Read the secret over stdin; only the derived public key may reach stdout.
guard let encoded = readLine(), let seed = Data(base64Encoded:encoded),
      let key = try? Curve25519.Signing.PrivateKey(rawRepresentation:seed) else { exit(1) }
print(key.publicKey.rawRepresentation.base64EncodedString())
