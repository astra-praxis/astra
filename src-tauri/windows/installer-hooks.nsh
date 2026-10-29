; Tauri NSIS 安装器钩子
; 支持的宏：NSIS_HOOK_PREINSTALL / POSTINSTALL / PREUNINSTALL / POSTUNINSTALL

!macro NSIS_HOOK_PREINSTALL
  DetailPrint "正在准备安装环境…"
!macroend

!macro NSIS_HOOK_POSTINSTALL
  DetailPrint "正在完成最后的配置…"
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  DetailPrint "正在清理用户数据…"
!macroend

!macro NSIS_HOOK_POSTUNINSTALL
  DetailPrint "卸载完成"
!macroend
