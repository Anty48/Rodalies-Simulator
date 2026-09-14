' ============================================================================
'  Rodalies Simulator - lanzador de escritorio (sin cmd, sin pestaña de navegador)
'  Arranca el servidor local OCULTO (sin ventana de consola), espera a que
'  responda y abre la interfaz en una VENTANA DE APLICACIÓN (Edge/Chrome en
'  modo --app), directamente en el menú principal (http://127.0.0.1:8080/).
'  Al cerrar la ventana, detiene el servidor.
'  Se ejecuta con wscript.exe, que no muestra ninguna consola.
' ============================================================================
Option Explicit

Dim sh, fso, base, exePath, url, port
Set sh  = CreateObject("WScript.Shell")
Set fso = CreateObject("Scripting.FileSystemObject")

base    = fso.GetParentFolderName(WScript.ScriptFullName)
exePath = base & "\target\release\rodalies-sim.exe"
port    = 8080
url     = "http://127.0.0.1:" & port & "/"

If Not fso.FileExists(exePath) Then
  MsgBox "No encuentro el ejecutable:" & vbCrLf & exePath & vbCrLf & vbCrLf & _
         "Compílalo primero con:  cargo build --release", vbCritical, "Rodalies Simulator"
  WScript.Quit 1
End If

' ¿Ya hay un servidor respondiendo? Si es así, no arrancamos otro ni lo mataremos.
Dim startedByUs
startedByUs = False
If Not ServerUp(url) Then
  sh.CurrentDirectory = base
  ' 0 = ventana oculta (sin consola);  False = no esperar (arranca en segundo plano)
  sh.Run """" & exePath & """ --no-open", 0, False
  startedByUs = True

  ' Espera a que el servidor esté listo (carga del GTFS): hasta ~30 s.
  Dim i
  For i = 1 To 60
    WScript.Sleep 500
    If ServerUp(url) Then Exit For
  Next
End If

' Abre la interfaz como ventana de aplicación (chromeless), no como pestaña.
Dim browser, profile, cmd
browser = FindBrowser()
profile = sh.ExpandEnvironmentStrings("%LOCALAPPDATA%") & "\RodaliesSimulator"

If browser <> "" Then
  cmd = """" & browser & """ --app=" & url & _
        " --user-data-dir=""" & profile & """ --window-size=1360,900 --window-position=center"
  ' 1 = ventana normal;  True = espera a que se cierre la ventana-app
  sh.Run cmd, 1, True
Else
  ' Sin Edge/Chrome: como último recurso, el navegador por defecto (sin modo app).
  sh.Run url, 1, False
  ' No podemos esperar de forma fiable al cierre; dejamos el servidor en marcha.
  WScript.Quit 0
End If

' Ventana cerrada: si arrancamos el servidor nosotros, lo apagamos.
If startedByUs Then
  sh.Run "taskkill /F /IM rodalies-sim.exe", 0, True
End If

WScript.Quit 0

' ---------------------------------------------------------------------------
' ¿El servidor responde en /health?
Function ServerUp(u)
  On Error Resume Next
  Dim http
  Set http = CreateObject("WinHttp.WinHttpRequest.5.1")
  http.SetTimeouts 800, 800, 800, 800
  http.Open "GET", u & "health", False
  http.Send
  ServerUp = (Err.Number = 0 And http.Status = 200)
  On Error GoTo 0
End Function

' Localiza msedge.exe o chrome.exe en las rutas habituales.
Function FindBrowser()
  Dim candidates, c
  candidates = Array( _
    sh.ExpandEnvironmentStrings("%ProgramFiles(x86)%") & "\Microsoft\Edge\Application\msedge.exe", _
    sh.ExpandEnvironmentStrings("%ProgramFiles%")      & "\Microsoft\Edge\Application\msedge.exe", _
    sh.ExpandEnvironmentStrings("%ProgramFiles%")      & "\Google\Chrome\Application\chrome.exe", _
    sh.ExpandEnvironmentStrings("%ProgramFiles(x86)%") & "\Google\Chrome\Application\chrome.exe", _
    sh.ExpandEnvironmentStrings("%LOCALAPPDATA%")      & "\Google\Chrome\Application\chrome.exe")
  FindBrowser = ""
  For Each c In candidates
    If fso.FileExists(c) Then
      FindBrowser = c
      Exit Function
    End If
  Next
End Function
