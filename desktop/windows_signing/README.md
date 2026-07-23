# Windows Code Signing

The Tauri release workflow signs Windows bundles with Azure Trusted Signing
through `trusted-signing-cli`.

## CI

[`tauri-app-ci.yml`](../../.github/workflows/tauri-app-ci.yml) enables signing
when these repository secrets are configured:

- `AZURE_CLIENT_ID`
- `AZURE_TENANT_ID`
- `AZURE_CLIENT_SECRET`

The endpoint, signing account, and certificate profile are supplied by the
workflow's Tauri `signCommand`. Credential values must not be committed.

## Local Single-File Signing

Install the CLI:

```powershell
cargo install trusted-signing-cli
```

Copy `.env.example` to the ignored `.env`, add the credential values, and load
them:

```powershell
. .\windows_signing\load-signing-env.ps1
```

Sign one binary or installer:

```powershell
.\windows_signing\sign-binary.ps1 -FilePath "path\to\artifact.exe"
```

The Tauri workflow is the source of truth for release signing. These local
scripts are diagnostic helpers, not a separate installer build pipeline.
