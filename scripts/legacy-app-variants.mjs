import { createHash } from 'node:crypto'

// These Store public keys are test identity material only. Shipping ZIPs must
// never contain a manifest key, and this project does not need Store private keys.
export const LEGACY_APP_VARIANTS = [
  {
    slug: 'paid',
    productName: 'JSTorrent',
    extensionId: 'anhdpjpojoipgpmfanmedjghaligalgb',
    baselineVersion: '2.4.4',
    candidateVersion: '2.4.5',
    archiveName: 'jstorrent-legacy-paid-2.4.5.zip',
    publicKey:
      'MIGfMA0GCSqGSIb3DQEBAQUAA4GNADCBiQKBgQDCvpsh4qvVEOcUZPeucJJ5VASn8fIOGsoQIXnIewzRcqi3Nwj/4WttouI8Fp2OlNxjH6rkaFOSaUPv5n0j20M7clmTjFPmJtbdKKBdVnE5g1jRpkzwMPMV8fpP5IyyTy0hSkK1FAWuxnlBmOMLSAeqCsVH4cYO9s2ilFMNMEG04wIDAQAB',
  },
  {
    slug: 'lite',
    productName: 'JSTorrent Lite',
    extensionId: 'abmohcnlldaiaodkpacnldcdnjjgldfh',
    baselineVersion: '2.4.12',
    candidateVersion: '2.4.13',
    archiveName: 'jstorrent-legacy-lite-2.4.13.zip',
    publicKey:
      'MIIBIjANBgkqhkiG9w0BAQEFAAOCAQ8AMIIBCgKCAQEAqVkeimcHmKsjHDtmoH19r2XG1bFf6Dks3e8s6l+8LrS5SMY0FlJqTVNq5o3XR9jv8xk3v7Fxw79P2UR1SYgY1lwifIofjvB3ROylKmu3OvVxYaiJmPUoeGicyrjs4vRF9NuyQuvV1ltCpyA9dFqpsAai7neT8vpjgELbfddoE3OeDQ3u7ztG/tGxVjpBP5B1XcKPIU1IsJwfPZcvoL7lwaOL+8t0LFdMnlNCzkO+WsKG313oOvV+D1Fz/EEu4d7VfDOvQOwJ+DSFupd0Q0edLYKgh4LsOp42UCFABr+kS3O9CoNTgg8CBVpGQe3n1Yd+/jj0VUfdX7ZrIM3ilt7VmQIDAQAB',
  },
]

export function extensionIdFromPublicKey(publicKey) {
  const digest = createHash('sha256').update(Buffer.from(publicKey, 'base64')).digest()
  let extensionId = ''
  for (const byte of digest.subarray(0, 16)) {
    extensionId += String.fromCharCode(97 + (byte >> 4))
    extensionId += String.fromCharCode(97 + (byte & 15))
  }
  return extensionId
}
