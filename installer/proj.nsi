; Windows installer for proj. Build it with scripts\build-installer.ps1, which passes:
;   /DVERSION=0.1.0  /DEXE=path\to\proj.exe  /DOUTFILE=dist\proj-setup-0.1.0.exe
;
; Installs per user (no admin prompt) into %LOCALAPPDATA%\Programs\proj, puts that
; folder on the user's PATH (via `proj path add`), adds a Start menu shortcut and an
; "Apps & features" entry. Settings in %APPDATA%\proj are kept on uninstall.

Unicode true
!include "MUI2.nsh"
!include "FileFunc.nsh"

!ifndef VERSION
  !error "Pass /DVERSION=x.y.z"
!endif
!ifndef EXE
  !error "Pass /DEXE=path\to\proj.exe"
!endif
!ifndef OUTFILE
  !define OUTFILE "proj-setup-${VERSION}.exe"
!endif

!define UNINSTALL_KEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\proj"

Name "proj"
OutFile "${OUTFILE}"
InstallDir "$LOCALAPPDATA\Programs\proj"
InstallDirRegKey HKCU "Software\proj" "InstallDir"
RequestExecutionLevel user
SetCompressor /SOLID lzma
ShowInstDetails nevershow
ShowUninstDetails nevershow

VIProductVersion "${VERSION}.0"
VIAddVersionKey "ProductName" "proj"
VIAddVersionKey "FileDescription" "proj installer"
VIAddVersionKey "ProductVersion" "${VERSION}"
VIAddVersionKey "FileVersion" "${VERSION}"
VIAddVersionKey "LegalCopyright" "Copyright (c) Lucas Sebastião"

; Finish page: two checkboxes, "start on login" (borrowing the readme option) and "run now".
!define MUI_FINISHPAGE_SHOWREADME ""
!define MUI_FINISHPAGE_SHOWREADME_TEXT "Start proj when I log in"
!define MUI_FINISHPAGE_SHOWREADME_FUNCTION EnableAutostart
!define MUI_FINISHPAGE_RUN "$INSTDIR\proj.exe"
!define MUI_FINISHPAGE_RUN_TEXT "Start proj now"

!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_PAGE_FINISH
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_LANGUAGE "English"

Function EnableAutostart
  nsExec::Exec '"$INSTDIR\proj.exe" autostart on'
  Pop $0
FunctionEnd

; A running copy locks proj.exe; stop it before replacing or deleting it.
!macro StopProj
  nsExec::Exec 'taskkill /F /IM proj.exe'
  Pop $0
  Sleep 500
!macroend

Section "proj"
  !insertmacro StopProj
  SetOutPath "$INSTDIR"
  File "/oname=proj.exe" "${EXE}"
  WriteUninstaller "$INSTDIR\uninstall.exe"

  nsExec::Exec '"$INSTDIR\proj.exe" path add'
  Pop $0

  CreateShortcut "$SMPROGRAMS\proj.lnk" "$INSTDIR\proj.exe"

  WriteRegStr HKCU "Software\proj" "InstallDir" "$INSTDIR"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "DisplayName" "proj"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "DisplayVersion" "${VERSION}"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "DisplayIcon" "$INSTDIR\proj.exe"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "InstallLocation" "$INSTDIR"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "UninstallString" '"$INSTDIR\uninstall.exe"'
  WriteRegStr HKCU "${UNINSTALL_KEY}" "QuietUninstallString" '"$INSTDIR\uninstall.exe" /S'
  WriteRegDWORD HKCU "${UNINSTALL_KEY}" "NoModify" 1
  WriteRegDWORD HKCU "${UNINSTALL_KEY}" "NoRepair" 1
  ${GetSize} "$INSTDIR" "/S=0K" $0 $1 $2
  IntFmt $0 "0x%08X" $0
  WriteRegDWORD HKCU "${UNINSTALL_KEY}" "EstimatedSize" "$0"
SectionEnd

Section "Uninstall"
  !insertmacro StopProj
  nsExec::Exec '"$INSTDIR\proj.exe" path remove'
  Pop $0
  nsExec::Exec '"$INSTDIR\proj.exe" autostart off'
  Pop $0

  Delete "$INSTDIR\proj.exe"
  Delete "$INSTDIR\uninstall.exe"
  RMDir "$INSTDIR"
  Delete "$SMPROGRAMS\proj.lnk"

  DeleteRegKey HKCU "${UNINSTALL_KEY}"
  DeleteRegKey HKCU "Software\proj"
SectionEnd
