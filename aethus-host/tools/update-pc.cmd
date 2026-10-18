@echo off
setlocal
python -I "%~dp0update_pc.py" %*
exit /b %ERRORLEVEL%
