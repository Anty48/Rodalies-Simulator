extends Control
## ════════════════════════════════════════════════════════════════════════════
##  MenuPrincipal.gd  —  Lógica del Menú Principal del simulador
## ════════════════════════════════════════════════════════════════════════════
##  Pensado para crecer: el selector de mapas se rellena a partir de un Array de
##  datos (hoy solo "Catalunya", mañana los que quieras) y todos los botones ya
##  tienen "ganchos" preparados para añadirles sonidos y animaciones más adelante.
## ════════════════════════════════════════════════════════════════════════════


# ─────────────────────────────────────────────────────────────────────────────
#  CATÁLOGO DE MAPAS (Data-Driven)
#  Añade aquí nuevas entradas y el menú las mostrará solo. Cada mapa apunta a la
#  escena que se cargará al pulsar "Jugar".
# ─────────────────────────────────────────────────────────────────────────────
var mapas_disponibles: Array = [
	{
		"id": "catalunya",
		"nombre": "Catalunya (Rodalies)",
		"escena": "res://escenas/MapaCatalunya.tscn",
	},
	# Ejemplo para el futuro (descomenta cuando crees la escena):
	# {
	#     "id": "valencia",
	#     "nombre": "València (Cercanías)",
	#     "escena": "res://escenas/MapaValencia.tscn",
	# },
]


# ─────────────────────────────────────────────────────────────────────────────
#  REFERENCIAS A NODOS DE LA ESCENA
#  Tras añadir este script al nodo raíz, arrastra cada nodo desde el árbol al
#  campo correspondiente del Inspector. Si dejas alguno vacío, el código lo
#  ignora sin romperse (así puedes empezar con un menú mínimo e ir ampliando).
# ─────────────────────────────────────────────────────────────────────────────
@export var selector_mapas: OptionButton
@export var boton_jugar: Button
@export var boton_ajustes: Button
@export var boton_salir: Button
@export var panel_ajustes: Control
@export var slider_volumen: HSlider
@export var check_pantalla_completa: CheckButton


func _ready() -> void:
	# La escena original dejaba el Control raíz con un tamaño/offset fijo y
	# pequeño (herencia de cuando se colocó a mano en el editor) — lo
	# expandimos a toda la ventana para poder decorar un fondo de verdad.
	# Los NodePath exportados ("VBoxContainer/OptionButton", etc.) se
	# resuelven por nombre, no por posición/tamaño, así que esto no rompe nada.
	set_anchors_preset(Control.PRESET_FULL_RECT)

	_decorar_fondo()
	_decorar_titulo()
	_decorar_botones()
	_decorar_panel_ajustes()

	_rellenar_selector_mapas()
	_conectar_controles()
	if panel_ajustes:
		panel_ajustes.visible = false   # el panel de ajustes empieza oculto
	# El juego arranca en pantalla completa (ver Project Settings > Display >
	# Window > Size > Mode). Sincronizamos el check con el modo REAL de la
	# ventana en vez de fiarnos de su valor guardado en la escena, así nunca
	# se desincroniza si el modo por defecto cambia en el futuro.
	if check_pantalla_completa:
		var modo := DisplayServer.window_get_mode()
		check_pantalla_completa.button_pressed = modo == DisplayServer.WINDOW_MODE_FULLSCREEN \
			or modo == DisplayServer.WINDOW_MODE_EXCLUSIVE_FULLSCREEN


# ─────────────────────────────────────────────────────────────────────────────
#  DECORACIÓN VISUAL (construida por código, sin depender de imágenes: fondo,
#  franja de acento con los colores reales de las 8 líneas, título y estilo
#  de botones — coherente con el resto del juego, que también construye toda
#  su interfaz por código, ver EstacionUI.gd/TrenesPanel.gd).
# ─────────────────────────────────────────────────────────────────────────────

## Mismos colores que Global.get_color_linea() (ver datos/lineas.json) — no
## podemos usar Global aquí porque el menú puede abrirse antes de que el
## autoload termine de cargar los JSON, y son solo 8 valores fijos.
const COLORES_LINEAS := [
	Color("#3A99D4"), Color("#48A647"), Color("#7DC36D"), Color("#2E7D32"),
	Color("#EC0021"), Color("#FFB21E"), Color("#DA91C0"), Color("#9A2587"),
]


func _decorar_fondo() -> void:
	var fondo := ColorRect.new()
	fondo.set_anchors_preset(Control.PRESET_FULL_RECT)
	fondo.color = Color(0.09, 0.10, 0.13)
	fondo.mouse_filter = Control.MOUSE_FILTER_IGNORE
	add_child(fondo)
	move_child(fondo, 0)

	# Franja de acento arriba del todo con los colores de las 8 líneas reales:
	# un guiño a la red sin necesitar ningún recurso de imagen.
	var franja := HBoxContainer.new()
	franja.set_anchors_preset(Control.PRESET_TOP_WIDE)
	franja.mouse_filter = Control.MOUSE_FILTER_IGNORE
	for color in COLORES_LINEAS:
		var trozo := ColorRect.new()
		trozo.color = color
		trozo.custom_minimum_size = Vector2(0, 6)
		trozo.size_flags_horizontal = Control.SIZE_EXPAND_FILL
		franja.add_child(trozo)
	add_child(franja)
	move_child(franja, 1)


func _decorar_titulo() -> void:
	var titulo := Label.new()
	titulo.text = "ROD4LIA"
	titulo.horizontal_alignment = HORIZONTAL_ALIGNMENT_CENTER
	titulo.add_theme_font_size_override("font_size", 40)
	titulo.add_theme_color_override("font_color", Color(0.94, 0.95, 0.98))
	titulo.set_anchors_preset(Control.PRESET_CENTER_TOP)
	titulo.offset_left = -320.0
	titulo.offset_right = 320.0
	titulo.offset_top = -50.0
	titulo.offset_bottom = 140.0
	add_child(titulo)


func _decorar_botones() -> void:
	if selector_mapas:
		selector_mapas.custom_minimum_size = Vector2(280, 42)
		selector_mapas.add_theme_font_size_override("font_size", 15)
	if boton_jugar:
		_aplicar_estilo_boton(boton_jugar, Color(0.16, 0.53, 0.32))
	if boton_ajustes:
		_aplicar_estilo_boton(boton_ajustes, Color(0.20, 0.24, 0.32))
	if boton_salir:
		_aplicar_estilo_boton(boton_salir, Color(0.47, 0.17, 0.17))


func _aplicar_estilo_boton(boton: Button, color_base: Color) -> void:
	boton.custom_minimum_size = Vector2(280, 48)
	boton.add_theme_font_size_override("font_size", 18)

	var normal := StyleBoxFlat.new()
	normal.bg_color = color_base
	normal.set_corner_radius_all(6)
	normal.content_margin_left = 14.0
	normal.content_margin_right = 14.0
	normal.content_margin_top = 8.0
	normal.content_margin_bottom = 8.0

	var hover := normal.duplicate() as StyleBoxFlat
	hover.bg_color = color_base.lightened(0.15)
	var pulsado := normal.duplicate() as StyleBoxFlat
	pulsado.bg_color = color_base.darkened(0.15)

	boton.add_theme_stylebox_override("normal", normal)
	boton.add_theme_stylebox_override("hover", hover)
	boton.add_theme_stylebox_override("pressed", pulsado)
	boton.add_theme_stylebox_override("focus", hover)


## El panel de ajustes venía como un Panel de fábrica sin fondo propio,
## anidado dentro del propio botón "Ajustes" — no tocamos esa estructura (los
## NodePath exportados dependen de ella), solo le damos aspecto de panel de
## verdad.
func _decorar_panel_ajustes() -> void:
	if panel_ajustes == null or not (panel_ajustes is Panel):
		return
	var sb := StyleBoxFlat.new()
	sb.bg_color = Color(0.07, 0.08, 0.10, 0.97)
	sb.set_corner_radius_all(8)
	sb.border_width_top = 1
	sb.border_width_bottom = 1
	sb.border_width_left = 1
	sb.border_width_right = 1
	sb.border_color = Color(1.0, 1.0, 1.0, 0.08)
	(panel_ajustes as Panel).add_theme_stylebox_override("panel", sb)


# ─────────────────────────────────────────────────────────────────────────────
#  CONSTRUCCIÓN DINÁMICA DEL MENÚ
# ─────────────────────────────────────────────────────────────────────────────

func _rellenar_selector_mapas() -> void:
	if selector_mapas == null:
		return
	selector_mapas.clear()
	for mapa in mapas_disponibles:
		selector_mapas.add_item(mapa["nombre"])
	# El OptionButton recuerda el ÍNDICE elegido; lo usaremos en "Jugar".


func _conectar_controles() -> void:
	# Conectamos por código cada control a su función. Así no dependemos de
	# conexiones hechas a mano en el editor (más fácil de mantener).
	# Además, hacemos que cada botón reaccione al pasar el ratón por encima
	# (`mouse_entered`) para el futuro efecto de hover.
	if boton_jugar:
		boton_jugar.pressed.connect(_on_jugar_pressed)
		boton_jugar.mouse_entered.connect(_on_button_hover.bind(boton_jugar))
	if boton_ajustes:
		boton_ajustes.pressed.connect(_on_ajustes_pressed)
		boton_ajustes.mouse_entered.connect(_on_button_hover.bind(boton_ajustes))
	if boton_salir:
		boton_salir.pressed.connect(_on_salir_pressed)
		boton_salir.mouse_entered.connect(_on_button_hover.bind(boton_salir))
	if slider_volumen:
		slider_volumen.value_changed.connect(_on_volumen_cambiado)
	if check_pantalla_completa:
		check_pantalla_completa.toggled.connect(_on_pantalla_completa_toggled)


# ─────────────────────────────────────────────────────────────────────────────
#  ACCIONES DE LOS BOTONES
# ─────────────────────────────────────────────────────────────────────────────

func _on_jugar_pressed() -> void:
	_on_button_pressed(boton_jugar)   # gancho para sonido/animación (ver abajo)

	# ¿Qué mapa eligió el jugador en el selector?
	var indice := 0
	if selector_mapas:
		indice = selector_mapas.selected
	if indice < 0 or indice >= mapas_disponibles.size():
		indice = 0

	var escena_destino: String = str(mapas_disponibles[indice]["escena"])
	print("[Menú] Cargando mapa: %s" % escena_destino)

	# Cargamos la escena del mapa. (Si todavía no existe, se creará en una fase
	# posterior; por eso comprobamos antes que el recurso exista.)
	if ResourceLoader.exists(escena_destino):
		get_tree().change_scene_to_file(escena_destino)
	else:
		push_warning("[Menú] Aún no existe la escena: " + escena_destino)


func _on_ajustes_pressed() -> void:
	_on_button_pressed(boton_ajustes)
	if panel_ajustes:
		panel_ajustes.visible = not panel_ajustes.visible   # mostrar/ocultar


func _on_salir_pressed() -> void:
	_on_button_pressed(boton_salir)
	get_tree().quit()


# ─────────────────────────────────────────────────────────────────────────────
#  AJUSTES BÁSICOS (sonido / gráficos)
# ─────────────────────────────────────────────────────────────────────────────

func _on_volumen_cambiado(valor: float) -> void:
	# Configura el HSlider en el editor con rango 0.0 – 1.0.
	# Convertimos ese 0–1 a decibelios para el bus de audio "Master".
	var db := linear_to_db(clampf(valor, 0.0001, 1.0))
	var bus := AudioServer.get_bus_index("Master")
	AudioServer.set_bus_volume_db(bus, db)


func _on_pantalla_completa_toggled(activado: bool) -> void:
	if activado:
		DisplayServer.window_set_mode(DisplayServer.WINDOW_MODE_FULLSCREEN)
	else:
		DisplayServer.window_set_mode(DisplayServer.WINDOW_MODE_WINDOWED)


# ─────────────────────────────────────────────────────────────────────────────
#  GANCHOS PARA EFECTOS FUTUROS  (sonido + animación de botones)
#  Hoy solo dejan un rastro en la consola. Mañana, rellena su interior.
# ─────────────────────────────────────────────────────────────────────────────

func _on_button_hover(boton: Button) -> void:
	# TODO: reproducir un sonido suave de "hover" y/o animar el botón.
	#       Ejemplo de animación con un Tween (agrandar ligeramente):
	#   var tween := create_tween()
	#   tween.tween_property(boton, "scale", Vector2(1.05, 1.05), 0.1)
	print("[Menú] Hover sobre: ", boton.text if boton else "—")


func _on_button_pressed(boton: Button) -> void:
	# TODO: reproducir el sonido de "clic" y/o una animación de pulsación.
	#       Ejemplo: tener un nodo AudioStreamPlayer hijo llamado "SonidoClic":
	#   $SonidoClic.play()
	print("[Menú] Pulsado: ", boton.text if boton else "—")
