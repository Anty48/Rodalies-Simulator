class_name HUD
extends CanvasLayer
## ════════════════════════════════════════════════════════════════════════════
##  HUD.gd  —  Interfaz superpuesta: volver al menú + reloj + velocidad
## ════════════════════════════════════════════════════════════════════════════

var _reloj: Label
var _kpi: Label
var _panel_trenes: TrenesPanel = null


## Le damos al HUD una referencia al panel de trenes para poder abrirlo/cerrarlo
## desde su botón (ver MapaCatalunya._ready()).
func configurar(panel_trenes: TrenesPanel) -> void:
	_panel_trenes = panel_trenes


func _ready() -> void:
	var panel := PanelContainer.new()
	panel.position = Vector2(12, 12)
	add_child(panel)

	var fila := HBoxContainer.new()
	fila.add_theme_constant_override("separation", 10)
	panel.add_child(fila)

	# Volver al menú principal.
	_crear_boton(fila, "← Menú", Callable(self, "_volver_al_menu"))
	fila.add_child(VSeparator.new())

	# Reloj.
	_reloj = Label.new()
	_reloj.add_theme_font_size_override("font_size", 22)
	_reloj.text = Global.hora_actual()
	fila.add_child(_reloj)

	fila.add_child(VSeparator.new())

	# Control de velocidad. Las etiquetas son "veces la velocidad BASE del
	# juego" (no el tiempo real): x1 es esa base (con la que arrancamos
	# siempre) y el resto son múltiplos/fracciones de ella.
	_crear_boton(fila, "Pausa", Callable(Global, "pausar"))
	_crear_boton(fila, "x0.05", Callable(Global, "establecer_velocidad").bind("X0.05"))
	_crear_boton(fila, "x0.5", Callable(Global, "establecer_velocidad").bind("X0.5"))
	_crear_boton(fila, "x1", Callable(Global, "establecer_velocidad").bind("X1"))
	_crear_boton(fila, "x2", Callable(Global, "establecer_velocidad").bind("X2"))
	_crear_boton(fila, "x5", Callable(Global, "establecer_velocidad").bind("X5"))
	_crear_boton(fila, "x25", Callable(Global, "establecer_velocidad").bind("X25"))

	fila.add_child(VSeparator.new())
	_crear_boton(fila, "Trenes", Callable(self, "_alternar_panel_trenes"))

	fila.add_child(VSeparator.new())

	# KPI de la red (minutos·pasajero de retraso acumulados): el objetivo es
	# minimizarlo. Se alimenta de Tren.asignar_servicio() cada vez que un tren
	# arranca un servicio con retraso real.
	_kpi = Label.new()
	_kpi.add_theme_font_size_override("font_size", 16)
	_kpi.modulate = Color(0.85, 0.87, 0.92)
	_actualizar_kpi(Global.kpi_pasajeros_minuto)
	fila.add_child(_kpi)

	Global.tiempo_actualizado.connect(_on_tiempo_actualizado)
	Global.kpi_actualizado.connect(_actualizar_kpi)


func _crear_boton(contenedor: Container, etiqueta: String, accion: Callable) -> void:
	var boton := Button.new()
	boton.text = etiqueta
	boton.pressed.connect(accion)
	contenedor.add_child(boton)


func _volver_al_menu() -> void:
	get_tree().change_scene_to_file("res://escenas/MenuPrincipal.tscn")


func _on_tiempo_actualizado(hora_texto: String, _segundos: float) -> void:
	_reloj.text = hora_texto


func _actualizar_kpi(kpi_pasajeros_minuto: float) -> void:
	_kpi.text = "KPI retraso: %s pax·min" % _formatear_millares(int(kpi_pasajeros_minuto))


## "12345" -> "12.345": solo para que el KPI (que puede llegar a cifras
## grandes en una jornada mala) se lea de un vistazo.
func _formatear_millares(n: int) -> String:
	var texto := str(n)
	var resultado := ""
	var restantes := texto.length()
	for c in texto:
		resultado += c
		restantes -= 1
		if restantes > 0 and restantes % 3 == 0:
			resultado += "."
	return resultado


func _alternar_panel_trenes() -> void:
	if _panel_trenes != null:
		_panel_trenes.alternar()
