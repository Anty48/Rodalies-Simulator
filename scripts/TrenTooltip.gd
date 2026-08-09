class_name TrenTooltip
extends Node2D
## ════════════════════════════════════════════════════════════════════════════
##  TrenTooltip.gd  —  Etiqueta flotante con destino + retraso al pasar el ratón
## ════════════════════════════════════════════════════════════════════════════
##  Antes el retraso se dibujaba SIEMPRE sobre el tren (un elemento fijo más en
##  pantalla). Ahora solo aparece al hacer hover, en un cuadro pequeño con el
##  destino y el retraso actual. `top_level = true` para que el texto se lea
##  siempre horizontal: si heredara la rotación del tren (que gira con la vía),
##  quedaría boca abajo cuando el tren circula hacia la izquierda.
## ════════════════════════════════════════════════════════════════════════════

const COLOR_FONDO := Color(0.08, 0.09, 0.11, 0.92)
const COLOR_TEXTO := Color(0.95, 0.95, 0.98)
const TAM_FUENTE := 12
const PAD := 6.0
const INTERLINEA := 14.0

var texto: String = ""

var _lineas: PackedStringArray = []


func _init() -> void:
	top_level = true
	visible = false
	z_index = 100


func fijar_texto(t: String) -> void:
	if t == texto:
		return
	texto = t
	_lineas = texto.split("\n")
	queue_redraw()


func _draw() -> void:
	if _lineas.is_empty():
		return
	var fuente := ThemeDB.fallback_font
	if fuente == null:
		return

	var ancho := 0.0
	for l in _lineas:
		ancho = maxf(ancho, fuente.get_string_size(l, HORIZONTAL_ALIGNMENT_LEFT, -1, TAM_FUENTE).x)
	var alto := INTERLINEA * _lineas.size()

	var caja := Rect2(Vector2(-ancho / 2.0 - PAD, -alto - PAD * 2.0), Vector2(ancho + PAD * 2.0, alto + PAD * 2.0))
	draw_rect(caja, COLOR_FONDO, true)

	for i in _lineas.size():
		var y := -alto - PAD + INTERLINEA * (i + 1) - 3.0
		draw_string(fuente, Vector2(-ancho / 2.0, y), _lineas[i],
			HORIZONTAL_ALIGNMENT_LEFT, -1, TAM_FUENTE, COLOR_TEXTO)
