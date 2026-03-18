; OpenClaw Node Widget NSIS Installer
; Build: makensis installer.nsi
; Requires: NSIS 3.x (https://nsis.sourceforge.io)

!include "MUI2.nsh"

; --- Metadata ---
!define PRODUCT_NAME "OpenClaw Node Widget"
!define PRODUCT_EXE "openclaw-node-widget.exe"
!define PRODUCT_PUBLISHER "Beck Yeh"
!define PRODUCT_URL "https://github.com/beckyeh8888/openclaw-node-widget-rs"
!define AUM_ID "OpenClaw.NodeWidget"

; Version is passed via /DPRODUCT_VERSION=x.y.z at build time.
; Default to 0.0.0 if not provided.
!ifndef PRODUCT_VERSION
  !define PRODUCT_VERSION "0.0.0"
!endif

!define INSTALL_DIR "$PROGRAMFILES\${PRODUCT_NAME}"
!define UNINSTALL_REG "Software\Microsoft\Windows\CurrentVersion\Uninstall\${PRODUCT_NAME}"

; --- General ---
Name "${PRODUCT_NAME} ${PRODUCT_VERSION}"
OutFile "openclaw-node-widget-${PRODUCT_VERSION}-setup.exe"
InstallDir "${INSTALL_DIR}"
RequestExecutionLevel admin

; --- UI ---
!define MUI_ICON "..\assets\icon_online.ico"
!define MUI_ABORTWARNING

!insertmacro MUI_PAGE_WELCOME
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_COMPONENTS
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_PAGE_FINISH

!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES

!insertmacro MUI_LANGUAGE "English"

; --- Installer Sections ---

Section "!${PRODUCT_NAME}" SecMain
  SectionIn RO ; Required, cannot be unchecked

  SetOutPath "$INSTDIR"
  File "..\target\release\openclaw-node-widget-rs.exe"
  Rename "$INSTDIR\openclaw-node-widget-rs.exe" "$INSTDIR\${PRODUCT_EXE}"

  ; Create Start Menu shortcut with AppUserModelID
  CreateDirectory "$SMPROGRAMS\${PRODUCT_NAME}"
  CreateShortCut "$SMPROGRAMS\${PRODUCT_NAME}\${PRODUCT_NAME}.lnk" "$INSTDIR\${PRODUCT_EXE}"

  ; Write AppUserModelID to registry for toast notifications
  WriteRegStr HKCU "Software\Classes\AppUserModelId\${AUM_ID}" "DisplayName" "${PRODUCT_NAME}"

  ; Write uninstaller
  WriteUninstaller "$INSTDIR\uninstall.exe"

  ; Add/Remove Programs entry
  WriteRegStr HKLM "${UNINSTALL_REG}" "DisplayName" "${PRODUCT_NAME}"
  WriteRegStr HKLM "${UNINSTALL_REG}" "UninstallString" '"$INSTDIR\uninstall.exe"'
  WriteRegStr HKLM "${UNINSTALL_REG}" "InstallLocation" "$INSTDIR"
  WriteRegStr HKLM "${UNINSTALL_REG}" "Publisher" "${PRODUCT_PUBLISHER}"
  WriteRegStr HKLM "${UNINSTALL_REG}" "URLInfoAbout" "${PRODUCT_URL}"
  WriteRegStr HKLM "${UNINSTALL_REG}" "DisplayVersion" "${PRODUCT_VERSION}"
  WriteRegDWORD HKLM "${UNINSTALL_REG}" "NoModify" 1
  WriteRegDWORD HKLM "${UNINSTALL_REG}" "NoRepair" 1

  ; Estimate size (in KB)
  SectionGetSize ${SecMain} $0
  WriteRegDWORD HKLM "${UNINSTALL_REG}" "EstimatedSize" $0
SectionEnd

Section "Desktop Shortcut" SecDesktop
  CreateShortCut "$DESKTOP\${PRODUCT_NAME}.lnk" "$INSTDIR\${PRODUCT_EXE}"
SectionEnd

; --- Section Descriptions ---
!insertmacro MUI_FUNCTION_DESCRIPTION_BEGIN
  !insertmacro MUI_DESCRIPTION_TEXT ${SecMain} "Install ${PRODUCT_NAME} to Program Files."
  !insertmacro MUI_DESCRIPTION_TEXT ${SecDesktop} "Create a shortcut on the Desktop."
!insertmacro MUI_FUNCTION_DESCRIPTION_END

; --- Uninstaller ---
Section "Uninstall"
  ; Remove files
  Delete "$INSTDIR\${PRODUCT_EXE}"
  Delete "$INSTDIR\uninstall.exe"
  RMDir "$INSTDIR"

  ; Remove Start Menu shortcuts
  Delete "$SMPROGRAMS\${PRODUCT_NAME}\${PRODUCT_NAME}.lnk"
  RMDir "$SMPROGRAMS\${PRODUCT_NAME}"

  ; Remove Desktop shortcut
  Delete "$DESKTOP\${PRODUCT_NAME}.lnk"

  ; Remove autostart registry entry
  DeleteRegValue HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "OpenClawNodeWidget"

  ; Remove AUMID registry
  DeleteRegKey HKCU "Software\Classes\AppUserModelId\${AUM_ID}"

  ; Remove Add/Remove Programs entry
  DeleteRegKey HKLM "${UNINSTALL_REG}"
SectionEnd
