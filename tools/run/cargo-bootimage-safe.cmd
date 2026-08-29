@echo off
setlocal
cargo bootimage 2>&1
exit /b %ERRORLEVEL%
