; CAPTURE Windows installer (NSIS 3, Modern UI 2).
;
; Wraps the tested portable payload unchanged: the files in PAYLOAD are exactly
; dist\CAPTURE-<version>\ (see scripts\package-portable.ps1), staged by
; scripts\build-installer.ps1. Same CAPTURE.exe, same engine, same settings layout.
;
; - Per-user install, no administrator rights. Any folder the user can write to, on any drive.
;   CAPTURE keeps its Data\ folder next to the app, so protected folders (Program Files,
;   Windows) are refused with an explanation instead of silently moving data elsewhere.
; - One installation: an existing install is updated in place (settings, recordings kept).
; - Optional desktop shortcut and Start with Windows; Start menu shortcut; uninstall entry.
; - Uninstall: "Remove CAPTURE" (keeps recordings) or "Remove CAPTURE and all its data"
;   (also CAPTURE's own recordings, after a confirmation that lists where they are).
;
; Command line (for IT/testing): setup.exe /S [/STARTUP=1] [/DESKTOP=1] [/D=<folder>]
;                                 "Uninstall CAPTURE.exe" /S [/ALLDATA]

Unicode true
ManifestDPIAware true
RequestExecutionLevel user
SetCompressor /SOLID lzma
CRCCheck on

!ifndef VERSION
  !error "Pass /DVERSION=x.y.z"
!endif
!ifndef PAYLOAD
  !error "Pass /DPAYLOAD=<staged payload folder>"
!endif
!ifndef OUTFILE
  !define OUTFILE "CAPTURE-${VERSION}-setup.exe"
!endif

!include MUI2.nsh
!include nsDialogs.nsh
!include LogicLib.nsh
!include FileFunc.nsh
!include WinMessages.nsh

!define APP "CAPTURE"
!define EXE "CAPTURE.exe"
!define UNINSTALLER "Uninstall CAPTURE.exe"
!define UNINST_KEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\CAPTURE"
!define RUN_KEY "Software\Microsoft\Windows\CurrentVersion\Run"
; The running app's message window (tauri-plugin-single-instance: "<identifier>-sic").
!define INSTANCE_CLASS "app.capture.local-sic"

Name "${APP}"
OutFile "${OUTFILE}"
InstallDir "$LOCALAPPDATA\Programs\CAPTURE"
InstallDirRegKey HKCU "${UNINST_KEY}" "InstallLocation"
BrandingText "CAPTURE ${VERSION}"
ShowInstDetails nevershow
ShowUninstDetails nevershow

VIProductVersion "${VERSION}.0"
VIAddVersionKey "ProductName" "CAPTURE"
VIAddVersionKey "FileDescription" "CAPTURE Setup"
VIAddVersionKey "FileVersion" "${VERSION}"
VIAddVersionKey "ProductVersion" "${VERSION}"
VIAddVersionKey "LegalCopyright" "CAPTURE"

!define MUI_ICON "${ICON}"
!define MUI_UNICON "${ICON}"
!define MUI_ABORTWARNING
!define MUI_WELCOMEPAGE_TITLE "Install CAPTURE ${VERSION}"
!define MUI_WELCOMEPAGE_TEXT "CAPTURE records your games and keeps the last moments ready to save with one key.$\r$\n$\r$\nIt installs into one folder, together with its settings. Recordings go to a folder you choose when CAPTURE first starts.$\r$\n$\r$\nIf CAPTURE is running, it will be closed. A recording in progress is saved first."
!insertmacro MUI_PAGE_WELCOME

!define MUI_DIRECTORYPAGE_TEXT_TOP "CAPTURE and its settings are kept together in this folder. Recordings are not stored here, so it doesn't need to be a large drive.$\r$\n$\r$\nChoose any folder you can write to. Program Files is not used, because CAPTURE keeps its settings next to the app."
!define MUI_PAGE_CUSTOMFUNCTION_LEAVE DirLeave
!insertmacro MUI_PAGE_DIRECTORY

Page custom OptionsPage OptionsLeave

!insertmacro MUI_PAGE_INSTFILES

!define MUI_FINISHPAGE_TITLE "CAPTURE is installed"
!define MUI_FINISHPAGE_TEXT "CAPTURE is in $INSTDIR.$\r$\n$\r$\nIt keeps running in the tray when you close its window. Quit it from the tray icon."
!define MUI_FINISHPAGE_RUN
!define MUI_FINISHPAGE_RUN_TEXT "Open CAPTURE now"
!define MUI_FINISHPAGE_RUN_FUNCTION LaunchApp
!insertmacro MUI_PAGE_FINISH

UninstPage custom un.ChoicePage un.ChoiceLeave
!insertmacro MUI_UNPAGE_INSTFILES

!insertmacro MUI_LANGUAGE "English"

Var WantDesktop
Var WantStartup
Var DesktopBox
Var StartupBox
Var RemoveAll
Var KeepRadio
Var AllRadio
Var Failed

; ---------------------------------------------------------------------------------------------
; Helpers

; Close a running CAPTURE the same way as Quit in its tray menu (in-progress saves finish).
!macro CLOSE_CAPTURE un
Function ${un}CloseCapture
  FindWindow $0 "${INSTANCE_CLASS}"
  ${If} $0 == 0
    Return
  ${EndIf}
  DetailPrint "Closing CAPTURE…"
  ${If} ${FileExists} "$INSTDIR\${EXE}"
    Exec '"$INSTDIR\${EXE}" --quit'
  ${EndIf}
  StrCpy $1 0
  ${Do}
    Sleep 500
    FindWindow $0 "${INSTANCE_CLASS}"
    ${If} $0 == 0
      Sleep 1000 ; let the engine and WebView2 release their files
      Return
    ${EndIf}
    IntOp $1 $1 + 1
    ${If} $1 >= 90
      MessageBox MB_RETRYCANCEL|MB_ICONEXCLAMATION "CAPTURE is still running. Quit it from its tray icon (Quit CAPTURE), then choose Retry." /SD IDCANCEL IDRETRY retry
      Abort
      retry:
      StrCpy $1 0
    ${EndIf}
  ${Loop}
FunctionEnd
!macroend
!insertmacro CLOSE_CAPTURE ""
!insertmacro CLOSE_CAPTURE "un."

; $R0 = "1" if the Run entry exists and starts this folder's CAPTURE.exe.
!macro STARTUP_IS_OURS un
Function ${un}StartupIsOurs
  StrCpy $R0 0
  ReadRegStr $R1 HKCU "${RUN_KEY}" "CAPTURE"
  StrLen $R2 '"$INSTDIR\'
  StrCpy $R3 $R1 $R2
  ${If} $R3 == '"$INSTDIR\'
    StrCpy $R0 1
  ${EndIf}
FunctionEnd
!macroend
!insertmacro STARTUP_IS_OURS ""
!insertmacro STARTUP_IS_OURS "un."

; ---------------------------------------------------------------------------------------------
; Install

Function .onInit
  StrCpy $WantDesktop 0
  StrCpy $WantStartup 0
  ; An existing install: update it in place, keep its choices.
  ReadRegStr $0 HKCU "${UNINST_KEY}" "InstallLocation"
  ${If} $0 != ""
  ${AndIf} ${FileExists} "$0\${EXE}"
    StrCpy $INSTDIR $0
    ReadRegStr $WantDesktop HKCU "${UNINST_KEY}" "DesktopShortcut"
    ${If} $WantDesktop == ""
      StrCpy $WantDesktop 0
    ${EndIf}
    Call StartupIsOurs
    StrCpy $WantStartup $R0
  ${EndIf}
  ${GetParameters} $R9
  ClearErrors
  ${GetOptions} $R9 "/STARTUP=" $R8
  ${IfNot} ${Errors}
    StrCpy $WantStartup $R8
  ${EndIf}
  ClearErrors
  ${GetOptions} $R9 "/DESKTOP=" $R8
  ${IfNot} ${Errors}
    StrCpy $WantDesktop $R8
  ${EndIf}
FunctionEnd

; $R0 = 1 if $INSTDIR is inside $R1.
Function IsInside
  StrCpy $R0 0
  ${If} $R1 == ""
    Return
  ${EndIf}
  StrLen $R2 "$R1\"
  StrCpy $R3 "$INSTDIR\" $R2
  ${If} $R3 == "$R1\"
    StrCpy $R0 1
  ${EndIf}
FunctionEnd

Function DirLeave
  ; Protected locations: CAPTURE could not keep its Data folder next to the app.
  ${ForEach} $R5 0 2 + 1
    ${If} $R5 == 0
      StrCpy $R1 $PROGRAMFILES64
    ${ElseIf} $R5 == 1
      StrCpy $R1 $PROGRAMFILES
    ${Else}
      StrCpy $R1 $WINDIR
    ${EndIf}
    Call IsInside
    ${If} $R0 == 1
      MessageBox MB_OK|MB_ICONEXCLAMATION "Windows protects $R1, so CAPTURE couldn't keep its settings next to the app there.$\r$\n$\r$\nChoose a folder you own, for example:$\r$\n    $LOCALAPPDATA\Programs\CAPTURE$\r$\n    D:\Apps\CAPTURE"
      Abort
    ${EndIf}
  ${Next}
  ; Not a whole drive.
  StrLen $0 $INSTDIR
  ${If} $0 <= 3
    MessageBox MB_OK|MB_ICONEXCLAMATION "Choose a folder for CAPTURE rather than the whole drive, for example $INSTDIR\CAPTURE."
    Abort
  ${EndIf}
  ; Only one installed copy: an existing installation elsewhere is updated where it is.
  ReadRegStr $1 HKCU "${UNINST_KEY}" "InstallLocation"
  ${If} $1 != ""
  ${AndIf} $1 != $INSTDIR
  ${AndIf} ${FileExists} "$1\${EXE}"
    MessageBox MB_OKCANCEL|MB_ICONINFORMATION "CAPTURE is already installed in$\r$\n    $1$\r$\n$\r$\nIt will be updated there; its settings and recordings are kept. (To move CAPTURE, uninstall it first.)" IDOK useExisting
    Abort
    useExisting:
    StrCpy $INSTDIR $1
  ${EndIf}
  ; A portable copy (or other files) already in this folder.
  ${If} ${FileExists} "$INSTDIR\${EXE}"
    ${IfNot} ${FileExists} "$INSTDIR\${UNINSTALLER}"
      MessageBox MB_OKCANCEL|MB_ICONINFORMATION "This folder holds a portable copy of CAPTURE. Installing here updates it and adds it to the Start menu and Installed apps. Its settings are kept." IDOK folderOk
      Abort
    ${EndIf}
  ${Else}
    ${DirState} $INSTDIR $0
    ${If} $0 == 1
      MessageBox MB_OKCANCEL|MB_ICONINFORMATION "This folder is not empty. CAPTURE adds its own files and leaves everything else there alone." IDOK folderOk
      Abort
    ${EndIf}
  ${EndIf}
  folderOk:
  ; Must be writable without administrator rights. Probe the folder, or its nearest existing
  ; parent, without creating any folder (nothing is left behind if setup is cancelled).
  StrCpy $2 $INSTDIR
  ${DoUntil} ${FileExists} "$2\*.*"
    ${GetParent} $2 $2
    ${If} $2 == ""
      ${Break}
    ${EndIf}
  ${Loop}
  ; A folder probe (not a file): at a drive root such as C:\ users may create folders but not
  ; files, and CAPTURE only needs to create its own folder there.
  StrCpy $3 "$2\.capture-write-test-$HWNDPARENT"
  CreateDirectory $3
  ${IfNot} ${FileExists} "$3\*.*"
  ${OrIf} $2 == ""
    MessageBox MB_OK|MB_ICONEXCLAMATION "CAPTURE can't write to $INSTDIR. Choose a folder you own, for example $LOCALAPPDATA\Programs\CAPTURE."
    Abort
  ${EndIf}
  RMDir $3
FunctionEnd

Function OptionsPage
  !insertmacro MUI_HEADER_TEXT "Options" "Everything else is set up in CAPTURE itself."
  nsDialogs::Create 1018
  Pop $0
  ${NSD_CreateCheckbox} 0 10u 100% 12u "Create a desktop shortcut"
  Pop $DesktopBox
  ${If} $WantDesktop == 1
    ${NSD_Check} $DesktopBox
  ${EndIf}
  ${NSD_CreateCheckbox} 0 34u 100% 12u "Start CAPTURE when I sign in to Windows"
  Pop $StartupBox
  ${If} $WantStartup == 1
    ${NSD_Check} $StartupBox
  ${EndIf}
  ${NSD_CreateLabel} 12u 48u 95% 24u "CAPTURE starts in the tray, ready to record. You can change this any time in CAPTURE's Settings."
  Pop $0
  ${NSD_CreateLabel} 0 84u 100% 36u "A Start menu shortcut is always added. Recording folder, automatic recording and microphone are chosen the first time CAPTURE opens."
  Pop $0
  nsDialogs::Show
FunctionEnd

Function OptionsLeave
  ${NSD_GetState} $DesktopBox $WantDesktop
  ${If} $WantDesktop == ${BST_CHECKED}
    StrCpy $WantDesktop 1
  ${Else}
    StrCpy $WantDesktop 0
  ${EndIf}
  ${NSD_GetState} $StartupBox $WantStartup
  ${If} $WantStartup == ${BST_CHECKED}
    StrCpy $WantStartup 1
  ${Else}
    StrCpy $WantStartup 0
  ${EndIf}
FunctionEnd

Function LaunchApp
  Exec '"$INSTDIR\${EXE}"'
FunctionEnd

Section "CAPTURE" SecMain
  SectionIn RO
  StrCpy $Failed ""
  Call CloseCapture

  SetOutPath $INSTDIR
  ; Program files from 0.3–0.4 portable folders (flat layout), replaced by Runtime\ and Resources\.
  Delete "$INSTDIR\capture-worker.exe"
  Delete "$INSTDIR\avcodec-62.dll"
  Delete "$INSTDIR\avformat-62.dll"
  Delete "$INSTDIR\avutil-60.dll"
  Delete "$INSTDIR\swresample-6.dll"
  Delete "$INSTDIR\swscale-9.dll"
  Delete "$INSTDIR\QUICKSTART.md"
  RMDir /r "$INSTDIR\licenses"
  ; The previous version's private runtime is replaced as a whole, so no obsolete DLLs remain.
  RMDir /r "$INSTDIR\Runtime"
  RMDir /r "$INSTDIR\Resources"

  File /r "${PAYLOAD}\*.*"
  WriteUninstaller "$INSTDIR\${UNINSTALLER}"

  ; Start menu (one shortcut, no folder) and optional desktop shortcut.
  CreateShortCut "$SMPROGRAMS\CAPTURE.lnk" "$INSTDIR\${EXE}" "" "$INSTDIR\${EXE}" 0 SW_SHOWNORMAL "" "Record games and save instant replays"
  ${If} $WantDesktop == 1
    CreateShortCut "$DESKTOP\CAPTURE.lnk" "$INSTDIR\${EXE}" "" "$INSTDIR\${EXE}" 0
  ${Else}
    Delete "$DESKTOP\CAPTURE.lnk"
  ${EndIf}

  ; Start with Windows: the same per-user Run entry CAPTURE's own setting uses.
  ${If} $WantStartup == 1
    WriteRegStr HKCU "${RUN_KEY}" "CAPTURE" '"$INSTDIR\${EXE}" --tray'
  ${Else}
    Call StartupIsOurs
    ${If} $R0 == 1
      DeleteRegValue HKCU "${RUN_KEY}" "CAPTURE"
    ${EndIf}
  ${EndIf}

  ; Installed apps entry (per user).
  WriteRegStr HKCU "${UNINST_KEY}" "DisplayName" "CAPTURE"
  WriteRegStr HKCU "${UNINST_KEY}" "DisplayVersion" "${VERSION}"
  WriteRegStr HKCU "${UNINST_KEY}" "Publisher" "CAPTURE"
  WriteRegStr HKCU "${UNINST_KEY}" "DisplayIcon" "$INSTDIR\${EXE},0"
  WriteRegStr HKCU "${UNINST_KEY}" "InstallLocation" "$INSTDIR"
  WriteRegStr HKCU "${UNINST_KEY}" "UninstallString" '"$INSTDIR\${UNINSTALLER}"'
  WriteRegStr HKCU "${UNINST_KEY}" "QuietUninstallString" '"$INSTDIR\${UNINSTALLER}" /S'
  WriteRegStr HKCU "${UNINST_KEY}" "DesktopShortcut" $WantDesktop
  WriteRegDWORD HKCU "${UNINST_KEY}" "NoModify" 1
  WriteRegDWORD HKCU "${UNINST_KEY}" "NoRepair" 1
  ${GetSize} "$INSTDIR\Runtime" "/S=0K" $0 $1 $2
  ${GetSize} "$INSTDIR" "/M=${EXE} /S=0K /G=0" $3 $1 $2
  IntOp $0 $0 + $3
  WriteRegDWORD HKCU "${UNINST_KEY}" "EstimatedSize" $0
SectionEnd

; ---------------------------------------------------------------------------------------------
; Uninstall

Function un.onInit
  StrCpy $RemoveAll 0
  ${GetParameters} $R9
  ClearErrors
  ${GetOptions} $R9 "/ALLDATA" $R8
  ${IfNot} ${Errors}
    StrCpy $RemoveAll 1 ; command-line confirmation for unattended removal
  ${EndIf}
FunctionEnd

Function un.ChoicePage
  !insertmacro MUI_HEADER_TEXT "Uninstall CAPTURE" "Choose what to remove."
  nsDialogs::Create 1018
  Pop $0
  ${NSD_CreateRadioButton} 0 6u 100% 12u "Remove CAPTURE"
  Pop $KeepRadio
  ${NSD_CreateLabel} 12u 20u 95% 24u "Removes the app, its settings, cache and logs, its shortcuts and its startup entry. Your recordings stay where they are."
  Pop $0
  ${NSD_CreateRadioButton} 0 52u 100% 12u "Remove CAPTURE and all its data"
  Pop $AllRadio
  ${NSD_CreateLabel} 12u 66u 95% 36u "Also removes the clips, sessions, screenshots and editor audio files CAPTURE made. You'll see where they are before anything is removed, and they go to the Recycle Bin. Other files in those folders are never touched."
  Pop $0
  ${If} $RemoveAll == 1
    ${NSD_Check} $AllRadio
  ${Else}
    ${NSD_Check} $KeepRadio
  ${EndIf}
  nsDialogs::Show
FunctionEnd

Function un.ChoiceLeave
  ${NSD_GetState} $AllRadio $0
  ${If} $0 == ${BST_CHECKED}
    ; CAPTURE lists exactly what it would remove and asks (6 = Yes).
    ExecWait '"$INSTDIR\${EXE}" --uninstall-ask-data' $1
    ${If} $1 == 6
      StrCpy $RemoveAll 1
    ${Else}
      ${NSD_Uncheck} $AllRadio
      ${NSD_Check} $KeepRadio
      StrCpy $RemoveAll 0
      Abort ; stay on this page
    ${EndIf}
  ${Else}
    StrCpy $RemoveAll 0
  ${EndIf}
FunctionEnd

!macro TRY_DELETE path
  Delete "${path}"
  ${If} ${FileExists} "${path}"
    StrCpy $Failed "$Failed$\r$\n    ${path}"
  ${EndIf}
!macroend

!macro TRY_RMDIR path
  RMDir /r "${path}"
  ${If} ${FileExists} "${path}\*.*"
    StrCpy $Failed "$Failed$\r$\n    ${path}"
  ${EndIf}
!macroend

Section "Uninstall"
  StrCpy $Failed ""
  Call un.CloseCapture

  ; CAPTURE's own files in the recording folders: temporary replay footage always, recordings
  ; only with "all its data". Done by CAPTURE itself, which knows the folders and checks every
  ; file is its own (see app/src-tauri/src/owned.rs).
  ${If} ${FileExists} "$INSTDIR\${EXE}"
    StrCpy $2 "temp"
    ${If} $RemoveAll == 1
      StrCpy $2 "all"
    ${EndIf}
    ; Interactive: CAPTURE shows any file it could not remove. Silent: only the exit code.
    ${If} ${Silent}
      ExecWait '"$INSTDIR\${EXE}" --uninstall-cleanup $2 --silent' $0
    ${Else}
      ExecWait '"$INSTDIR\${EXE}" --uninstall-cleanup $2' $0
    ${EndIf}
    ${If} $0 != 0
      StrCpy $Failed "$Failed$\r$\n    some files in your recording folders"
    ${EndIf}
  ${EndIf}

  ; Startup entry, only if it starts this copy.
  Call un.StartupIsOurs
  ${If} $R0 == 1
    DeleteRegValue HKCU "${RUN_KEY}" "CAPTURE"
  ${EndIf}

  ; Shortcuts this installer made.
  Delete "$SMPROGRAMS\CAPTURE.lnk"
  ReadRegStr $0 HKCU "${UNINST_KEY}" "DesktopShortcut"
  ${If} $0 == 1
    Delete "$DESKTOP\CAPTURE.lnk"
  ${EndIf}

  ; Program files, private runtime, settings, cache and logs. Never a recursive delete of
  ; $INSTDIR itself: it is removed only if nothing else is left in it.
  !insertmacro TRY_DELETE "$INSTDIR\${EXE}"
  !insertmacro TRY_DELETE "$INSTDIR\manifest.json"
  !insertmacro TRY_RMDIR "$INSTDIR\Runtime"
  !insertmacro TRY_RMDIR "$INSTDIR\Resources"
  !insertmacro TRY_RMDIR "$INSTDIR\Data"
  Delete "$INSTDIR\${UNINSTALLER}"
  RMDir "$INSTDIR"

  DeleteRegKey HKCU "${UNINST_KEY}"

  ${If} $Failed != ""
    MessageBox MB_OK|MB_ICONEXCLAMATION "CAPTURE was uninstalled, but these could not be removed (another program may be using them). You can delete them yourself:$Failed" /SD IDOK
  ${EndIf}
SectionEnd
