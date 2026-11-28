Unicode true
!include "MUI2.nsh"
!include "LogicLib.nsh"
!include "FileFunc.nsh"

!define APP_NAME "Zero 智能内存工作台"
!define APP_EXE "zero-scanner.exe"
!define APP_VERSION "1.1.0"
!define PUBLISHER "highho"
!define APP_URL "https://github.com/highho/zero-scanner"
!define UNINST_KEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\ZeroScanner"

Name "${APP_NAME}"
OutFile "..\dist\zero-scanner-${APP_VERSION}-setup-x64.exe"
InstallDir "$LOCALAPPDATA\Programs\Zero"
InstallDirRegKey HKCU "${UNINST_KEY}" "InstallLocation"
RequestExecutionLevel user
SetCompressor /SOLID lzma

VIProductVersion "1.0.0.0"
VIAddVersionKey /LANG=2052 "ProductName" "${APP_NAME}"
VIAddVersionKey /LANG=2052 "FileDescription" "Zero 安装程序"
VIAddVersionKey /LANG=2052 "FileVersion" "${APP_VERSION}"
VIAddVersionKey /LANG=2052 "ProductVersion" "${APP_VERSION}"
VIAddVersionKey /LANG=2052 "CompanyName" "${PUBLISHER}"
VIAddVersionKey /LANG=2052 "LegalCopyright" "MIT License"

!define MUI_ICON "zero.ico"
!define MUI_UNICON "zero.ico"
!define MUI_ABORTWARNING
!define MUI_FINISHPAGE_RUN "$INSTDIR\${APP_EXE}"
!define MUI_FINISHPAGE_RUN_TEXT "现在启动 Zero"
!define MUI_WELCOMEPAGE_TITLE "安装 Zero 智能内存工作台"
!define MUI_WELCOMEPAGE_TEXT "Zero 是一个本机推理的内存工作台：扫描、定位、观察、修改，全程离线。$\r$\n$\r$\n它会被装到你的用户目录（不需要管理员权限），并创建桌面与开始菜单快捷方式。"

!insertmacro MUI_PAGE_WELCOME
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_PAGE_FINISH
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_LANGUAGE "SimpChinese"

Section "Zero" SecMain
  SetOutPath "$INSTDIR"
  File "..\dist\zero-scanner.exe"
  File "zero.ico"
  File "..\README.md"
  File "..\LICENSE"

  CreateShortCut "$DESKTOP\Zero.lnk" "$INSTDIR\${APP_EXE}" "" "$INSTDIR\zero.ico" 0 SW_SHOWNORMAL "" "Zero · 本机内存工作台"
  CreateDirectory "$SMPROGRAMS\Zero"
  CreateShortCut "$SMPROGRAMS\Zero\Zero.lnk" "$INSTDIR\${APP_EXE}" "" "$INSTDIR\zero.ico" 0
  CreateShortCut "$SMPROGRAMS\Zero\卸载 Zero.lnk" "$INSTDIR\uninstall.exe"

  WriteUninstaller "$INSTDIR\uninstall.exe"

  WriteRegStr HKCU "${UNINST_KEY}" "DisplayName" "${APP_NAME}"
  WriteRegStr HKCU "${UNINST_KEY}" "DisplayVersion" "${APP_VERSION}"
  WriteRegStr HKCU "${UNINST_KEY}" "Publisher" "${PUBLISHER}"
  WriteRegStr HKCU "${UNINST_KEY}" "DisplayIcon" "$INSTDIR\zero.ico"
  WriteRegStr HKCU "${UNINST_KEY}" "InstallLocation" "$INSTDIR"
  WriteRegStr HKCU "${UNINST_KEY}" "UninstallString" "$\"$INSTDIR\uninstall.exe$\""
  WriteRegStr HKCU "${UNINST_KEY}" "QuietUninstallString" "$\"$INSTDIR\uninstall.exe$\" /S"
  WriteRegStr HKCU "${UNINST_KEY}" "URLInfoAbout" "${APP_URL}"
  WriteRegDWORD HKCU "${UNINST_KEY}" "NoModify" 1
  WriteRegDWORD HKCU "${UNINST_KEY}" "NoRepair" 1

  ${GetSize} "$INSTDIR" "/S=0K" $0 $1 $2
  IntFmt $0 "0x%08X" $0
  WriteRegDWORD HKCU "${UNINST_KEY}" "EstimatedSize" "$0"
SectionEnd

Section "Uninstall"
  Delete "$DESKTOP\Zero.lnk"
  Delete "$SMPROGRAMS\Zero\Zero.lnk"
  Delete "$SMPROGRAMS\Zero\卸载 Zero.lnk"
  RMDir "$SMPROGRAMS\Zero"

  Delete "$INSTDIR\zero-scanner.exe"
  Delete "$INSTDIR\zero.ico"
  Delete "$INSTDIR\README.md"
  Delete "$INSTDIR\LICENSE"
  Delete "$INSTDIR\uninstall.exe"
  ; zero-workspace.json 是用户数据（地址列表等），故意保留：目录非空时不会被删掉
  RMDir "$INSTDIR"

  DeleteRegKey HKCU "${UNINST_KEY}"
SectionEnd
