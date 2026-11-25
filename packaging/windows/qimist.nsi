; Qimist Windows installer (NSIS). Built by scripts/package.sh or the release
; workflow:  makensis -DVERSION=0.1.0 -DSRC=<folder with the built files> qimist.nsi
Unicode true
!include "MUI2.nsh"

!ifndef VERSION
  !define VERSION "0.1.0"
!endif
!ifndef SRC
  !define SRC "..\..\dist\windows"
!endif
!ifndef OUT
  !define OUT "Qimist-${VERSION}-windows-setup.exe"
!endif

Name "Qimist ${VERSION}"
OutFile "${OUT}"
InstallDir "$PROGRAMFILES64\Qimist"
RequestExecutionLevel admin
SetCompressor /SOLID lzma

!define MUI_ABORTWARNING
!insertmacro MUI_PAGE_WELCOME
!insertmacro MUI_PAGE_LICENSE "${SRC}\LICENSE.txt"
!insertmacro MUI_PAGE_COMPONENTS
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!define MUI_FINISHPAGE_RUN "$INSTDIR\Qimist.exe"
!define MUI_FINISHPAGE_SHOWREADME "$INSTDIR\MANUAL.html"
!define MUI_FINISHPAGE_SHOWREADME_TEXT "Open the manual"
!insertmacro MUI_PAGE_FINISH
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_LANGUAGE "English"

Section "Qimist app" SecApp
  SectionIn RO
  SetOutPath "$INSTDIR"
  File /oname=Qimist.exe "${SRC}\qimist-standalone.exe"
  File "${SRC}\MANUAL.html"
  File "${SRC}\LICENSE.txt"
  CreateDirectory "$SMPROGRAMS\Qimist"
  CreateShortcut "$SMPROGRAMS\Qimist\Qimist.lnk" "$INSTDIR\Qimist.exe"
  CreateShortcut "$SMPROGRAMS\Qimist\Qimist Manual.lnk" "$INSTDIR\MANUAL.html"
  CreateShortcut "$DESKTOP\Qimist.lnk" "$INSTDIR\Qimist.exe"
  WriteUninstaller "$INSTDIR\Uninstall.exe"
  WriteRegStr HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\Qimist" "DisplayName" "Qimist"
  WriteRegStr HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\Qimist" "DisplayVersion" "${VERSION}"
  WriteRegStr HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\Qimist" "Publisher" "Qimist"
  WriteRegStr HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\Qimist" "UninstallString" '"$INSTDIR\Uninstall.exe"'
SectionEnd

Section "VST3 plugin" SecVst3
  SetOutPath "$COMMONFILES64\VST3"
  File /r "${SRC}\Qimist.vst3"
SectionEnd

Section "CLAP plugin" SecClap
  SetOutPath "$COMMONFILES64\CLAP"
  File "${SRC}\Qimist.clap"
SectionEnd

Section "Uninstall"
  RMDir /r "$COMMONFILES64\VST3\Qimist.vst3"
  Delete "$COMMONFILES64\CLAP\Qimist.clap"
  Delete "$DESKTOP\Qimist.lnk"
  RMDir /r "$SMPROGRAMS\Qimist"
  RMDir /r "$INSTDIR"
  DeleteRegKey HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\Qimist"
SectionEnd
