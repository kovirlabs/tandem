@echo off
rem Builds and installs Tandem on this Windows PC. See scripts\windows\setup.ps1 for details.
rem Options: install-windows.bat -NoInstall    (build the installer only)
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0scripts\windows\setup.ps1" %*
echo.
pause
