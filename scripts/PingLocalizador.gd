class_name PingLocalizador
extends Node2D
## ════════════════════════════════════════════════════════════════════════════
##  PingLocalizador.gd  —  Efecto de "foco" temporal sobre un tren localizado
## ════════════════════════════════════════════════════════════════════════════
##  Lo usa TrenesPanel al pulsar "Ir al tren": un rectángulo concéntrico al
##  del propio tren que arranca varias veces más grande y se va encogiendo
##  hasta desaparecer justo al alcanzar el tamaño EXACTO del rectángulo que
##  dibuja Tren._draw() (Tren.LARGO x Tren.ANCHO) — así el ojo "cae" sobre el
##  tren en vez de tener que buscarlo.
##
##  Se añade como HIJO del propio Tren (ver TrenesPanel._ir_al_tren): así
##  hereda automáticamente su posición, rotación y la escala 1/zoom constante
##  en pantalla, sin tener que duplicar ninguna de esas cuentas aquí.
## ════════════════════════════════════════════════════════════════════════════

const FACTOR_INICIAL := 4.0
const DURACION_SEG := 1.0
const COLOR := Color(1.0, 0.9, 0.3, 1.0)
const GROSOR := 2.0

var _factor := FACTOR_INICIAL


func _ready() -> void:
	var tween := create_tween()
	tween.tween_method(_fijar_factor, FACTOR_INICIAL, 1.0, DURACION_SEG).set_trans(Tween.TRANS_CUBIC).set_ease(Tween.EASE_OUT)
	tween.tween_callback(queue_free)


func _fijar_factor(f: float) -> void:
	_factor = f
	queue_redraw()


func _draw() -> void:
	var largo := Tren.LARGO * _factor
	var ancho := Tren.ANCHO * _factor
	var rect := Rect2(-largo / 2.0, -ancho / 2.0, largo, ancho)
	# Se desvanece a la vez que se encoge, para que el final (tamaño exacto del
	# tren) quede limpio en vez de dejar un marco amarillo pegado encima.
	var t := clampf((_factor - 1.0) / (FACTOR_INICIAL - 1.0), 0.0, 1.0)
	var color := COLOR
	color.a = t
	draw_rect(rect, color, false, GROSOR)
