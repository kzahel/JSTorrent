; Kill JSTorrent processes before install/uninstall to prevent
; "two instances running" when upgrading manually.

!macro _KillJSTorrentProcesses
  ; Kill main app
  nsis_tauri_utils::FindProcess "JSTorrent.exe" $R0
  ${If} $R0 = 0
    nsis_tauri_utils::KillProcess "JSTorrent.exe" $R0
  ${EndIf}

  ; Kill the native host sidecar
  nsis_tauri_utils::FindProcess "jstorrent-host.exe" $R0
  ${If} $R0 = 0
    nsis_tauri_utils::KillProcess "jstorrent-host.exe" $R0
  ${EndIf}

  ; Kill sidecar: io-daemon
  nsis_tauri_utils::FindProcess "jstorrent-io-daemon.exe" $R0
  ${If} $R0 = 0
    nsis_tauri_utils::KillProcess "jstorrent-io-daemon.exe" $R0
  ${EndIf}

  ; Give processes time to exit
  Sleep 1000
!macroend

!include "WordFunc.nsh"

!macro NSIS_HOOK_PREINSTALL
  !insertmacro _KillJSTorrentProcesses
!macroend

; Register native messaging host for Chrome/Chromium browsers at install time
; (mirrors native_host.rs register_windows_browsers)
!macro NSIS_HOOK_POSTINSTALL
  ; Find the host sidecar binary (triple-suffixed)
  FindFirst $0 $1 "$INSTDIR\jstorrent-host-*.exe"
  FindClose $0

  ${If} $1 != ""
    ; Full path to host binary
    StrCpy $2 "$INSTDIR\$1"

    ; Escape backslashes for JSON (\ -> \\)
    ${WordReplace} $2 "\" "\\" "+" $3

    ; Create manifest directory
    CreateDirectory "$LOCALAPPDATA\com.jstorrent.desktop"

    ; Write manifest JSON
    FileOpen $4 "$LOCALAPPDATA\com.jstorrent.desktop\com.jstorrent.native.json" w
    FileWrite $4 '{$\r$\n'
    FileWrite $4 '  "name": "com.jstorrent.native",$\r$\n'
    FileWrite $4 '  "description": "JSTorrent Native Messaging Host",$\r$\n'
    FileWrite $4 '  "path": "$3",$\r$\n'
    FileWrite $4 '  "type": "stdio",$\r$\n'
    FileWrite $4 '  "allowed_origins": [$\r$\n'
    FileWrite $4 '    "chrome-extension://dbokmlpefliilbjldladbimlcfgbolhk/",$\r$\n'
    FileWrite $4 '    "chrome-extension://opkmhecbhgngcbglpcdfmnomkffenapc/"$\r$\n'
    FileWrite $4 '  ]$\r$\n'
    FileWrite $4 '}'
    FileClose $4

    ; Manifest path for registry
    StrCpy $5 "$LOCALAPPDATA\com.jstorrent.desktop\com.jstorrent.native.json"

    ; Register with browsers (matching native_host.rs and POSTUNINSTALL cleanup)
    WriteRegStr HKCU "Software\Google\Chrome\NativeMessagingHosts\com.jstorrent.native" "" $5
    WriteRegStr HKCU "Software\Chromium\NativeMessagingHosts\com.jstorrent.native" "" $5
    WriteRegStr HKCU "Software\BraveSoftware\Brave-Browser\NativeMessagingHosts\com.jstorrent.native" "" $5
    WriteRegStr HKCU "Software\Microsoft\Edge\NativeMessagingHosts\com.jstorrent.native" "" $5
  ${EndIf}
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  !insertmacro _KillJSTorrentProcesses
!macroend

; Clean up native messaging host registration on uninstall
!macro NSIS_HOOK_POSTUNINSTALL
  ; Remove Chrome native messaging host registry keys
  DeleteRegKey HKCU "Software\Google\Chrome\NativeMessagingHosts\com.jstorrent.native"
  DeleteRegKey HKCU "Software\Chromium\NativeMessagingHosts\com.jstorrent.native"
  DeleteRegKey HKCU "Software\BraveSoftware\Brave-Browser\NativeMessagingHosts\com.jstorrent.native"
  DeleteRegKey HKCU "Software\Microsoft\Edge\NativeMessagingHosts\com.jstorrent.native"

  ; Remove manifest JSON from app data directory
  Delete "$LOCALAPPDATA\com.jstorrent.desktop\com.jstorrent.native.json"
!macroend
