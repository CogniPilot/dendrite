@echo off
title Dendrite Viewer
cd /d "%~dp0"
echo Starting Dendrite...
echo.
echo The web interface will open in your browser automatically.
echo Press Ctrl+C to stop the server.
echo.
dendrite.exe --open
