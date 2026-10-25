call "C:\Program Files\Microsoft Visual Studio\18\Community\VC\Auxiliary\Build\vcvars64.bat" >nul 2>&1
cd /d "C:\Users\holwe\Desktop\496E~1\PROXIM~1"
cl.exe -nologo -MD -Z7 -I deps\dist_windows\include -W4 -c lib\encode_video.c -Fotest_c.obj
echo CL_EXIT=%ERRORLEVEL%
