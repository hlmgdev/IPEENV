!macro NSIS_HOOK_POSTUNINSTALL
  RMDir /r "$INSTDIR"
!macroend
