; Tauri's default helper skips shortcut creation during updates. Restore a
; missing Start menu entry on every install/update, unless /NS was requested.
; Keep the default root Programs location so Windows search finds Yas Mail.
!macro NSIS_HOOK_POSTINSTALL
  ${If} $NoShortcutMode != 1
    CreateShortcut "$SMPROGRAMS\${PRODUCTNAME}.lnk" "$INSTDIR\${MAINBINARYNAME}.exe" "" "$INSTDIR\${MAINBINARYNAME}.exe" 0
    !insertmacro SetLnkAppUserModelId "$SMPROGRAMS\${PRODUCTNAME}.lnk"
  ${EndIf}
!macroend
