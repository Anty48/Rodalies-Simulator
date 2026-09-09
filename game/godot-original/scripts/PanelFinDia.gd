class_name PanelFinDia
extends CanvasLayer
## ════════════════════════════════════════════════════════════════════════════
##  PanelFinDia.gd  —  Cuadro de "FIN DEL DÍA" (00:00)
## ════════════════════════════════════════════════════════════════════════════
##  Escucha Global.jornada_finalizada (se dispara UNA vez, cuando el reloj
##  llega a las 00:00 — ver Global._process). Al saltar: pausa la partida (si
##  no lo hiciéramos, los trenes seguirían circulando indefinidamente de
##  fondo, ya que Tren.gd no consulta para nada Global.jornada_terminada) y
##  muestra un resumen de las estadísticas del día con dos salidas:
##  "Repetir el día" (recarga la escena entera — más simple y fiable que
##  intentar resetear a mano el estado de cada Tren/ScheduleManager) o
##  "Menú principal".
## ════════════════════════════════════════════════════════════════════════════

const ESCENA_MENU := "res://escenas/MenuPrincipal.tscn"

var _trenes: Array = []
var _fondo: ColorRect
var _overlay: CenterContainer
var _panel: PanelContainer


func configurar(trenes: Array) -> void:
	_trenes = trenes


func _ready() -> void:
	layer = 20   # por encima de HUD, EstacionUI (10) y TrenesPanel
	Global.jornada_finalizada.connect(_mostrar)

	# Fondo oscuro semitransparente para que se note que el juego ha pasado a
	# un estado modal (mismo criterio que un diálogo de "game over" cualquiera).
	_fondo = ColorRect.new()
	_fondo.set_anchors_preset(Control.PRESET_FULL_RECT)
	_fondo.color = Color(0.0, 0.0, 0.0, 0.55)
	_fondo.visible = false
	add_child(_fondo)

	_overlay = CenterContainer.new()
	_overlay.set_anchors_preset(Control.PRESET_FULL_RECT)
	_overlay.visible = false
	add_child(_overlay)

	_panel = PanelContainer.new()
	var sb := StyleBoxFlat.new()
	sb.bg_color = Color(0.09, 0.10, 0.12, 0.98)
	sb.set_corner_radius_all(12)
	sb.content_margin_left = 32.0
	sb.content_margin_right = 32.0
	sb.content_margin_top = 24.0
	sb.content_margin_bottom = 24.0
	sb.border_width_top = 2
	sb.border_color = Color(0.95, 0.75, 0.25, 0.85)
	_panel.add_theme_stylebox_override("panel", sb)
	_overlay.add_child(_panel)


func _mostrar() -> void:
	Global.pausar()

	for c in _panel.get_children():
		_panel.remove_child(c)
		c.queue_free()

	var vb := VBoxContainer.new()
	vb.add_theme_constant_override("separation", 12)
	vb.custom_minimum_size = Vector2(360, 0)
	_panel.add_child(vb)

	var titulo := Label.new()
	titulo.text = "FIN DEL DÍA"
	titulo.horizontal_alignment = HORIZONTAL_ALIGNMENT_CENTER
	titulo.add_theme_font_size_override("font_size", 26)
	vb.add_child(titulo)

	var subtitulo := Label.new()
	subtitulo.text = "Jornada 05:00 – 00:00 completada"
	subtitulo.horizontal_alignment = HORIZONTAL_ALIGNMENT_CENTER
	subtitulo.modulate = Color(0.72, 0.74, 0.8)
	vb.add_child(subtitulo)

	vb.add_child(HSeparator.new())

	var en_hora := 0
	var suma_retraso_seg := 0.0
	for t in _trenes:
		var tren := t as Tren
		if tren == null:
			continue
		if tren.retraso_seg() <= 60.0:
			en_hora += 1
		suma_retraso_seg += tren.retraso_seg()
	var total := _trenes.size()
	var pct := (100.0 * en_hora / float(total)) if total > 0 else 0.0
	var retraso_medio_min := (suma_retraso_seg / 60.0) / float(total) if total > 0 else 0.0

	_fila_stat(vb, "Retraso medio por tren", "%.1f min" % retraso_medio_min)
	_fila_stat(vb, "Pasajeros afectados", "%d" % Global.total_pasajeros_afectados)
	_fila_stat(vb, "Impacto al usuario (pasajeros·min de retraso)", "%s" % _formatear_millares(int(Global.kpi_pasajeros_minuto)))
	_fila_stat(vb, "Trenes puntuales al cierre", "%d / %d (%.0f%%)" % [en_hora, total, pct])

	vb.add_child(HSeparator.new())

	var botones := HBoxContainer.new()
	botones.alignment = BoxContainer.ALIGNMENT_CENTER
	botones.add_theme_constant_override("separation", 14)
	vb.add_child(botones)

	var btn_reiniciar := Button.new()
	btn_reiniciar.text = "Repetir el día"
	btn_reiniciar.pressed.connect(func() -> void: get_tree().reload_current_scene())
	botones.add_child(btn_reiniciar)

	var btn_menu := Button.new()
	btn_menu.text = "Menú principal"
	btn_menu.pressed.connect(func() -> void: get_tree().change_scene_to_file(ESCENA_MENU))
	botones.add_child(btn_menu)

	_fondo.visible = true
	_overlay.visible = true


func _fila_stat(vb: VBoxContainer, etiqueta: String, valor: String) -> void:
	var fila := HBoxContainer.new()
	fila.add_theme_constant_override("separation", 24)
	var lbl := Label.new()
	lbl.text = etiqueta
	lbl.custom_minimum_size = Vector2(230, 0)
	lbl.modulate = Color(0.75, 0.77, 0.82)
	fila.add_child(lbl)
	var val := Label.new()
	val.text = valor
	val.add_theme_font_size_override("font_size", 16)
	fila.add_child(val)
	vb.add_child(fila)


func _formatear_millares(n: int) -> String:
	var s := str(n)
	var salida := ""
	var contador := 0
	for i in range(s.length() - 1, -1, -1):
		salida = s[i] + salida
		contador += 1
		if contador % 3 == 0 and i > 0:
			salida = "." + salida
	return salida
