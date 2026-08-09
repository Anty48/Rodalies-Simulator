class_name Estacion
extends Area2D
## ════════════════════════════════════════════════════════════════════════════
##  Estacion.gd  —  Estación: punto de tamaño de pantalla constante, con hover
##                  (muestra nombre y vías) y CLIC (abre el panel de enclavamiento)
## ════════════════════════════════════════════════════════════════════════════
##  Ahora es un Area2D para que Godot detecte de forma fiable cuándo el ratón
##  está encima (hover) y cuándo se hace clic. El círculo de colisión escala con
##  el nodo (scale = 1/zoom), así que la zona "pulsable" mide siempre lo mismo en
##  pantalla, da igual el zoom.
## ════════════════════════════════════════════════════════════════════════════

const RADIO_PUNTO := 4.0
const RADIO_PUNTO_TERMINAL := 7.0                 # estaciones terminal de alguna línea: punto más grande
const RADIO_HOVER := 14.0                         # px de pantalla (radio de colisión)
const SEPARACION_VIAS := 5.0
const LARGO_VIA := 26.0
const COLOR_PUNTO := Color(0.82, 0.85, 0.92)
const COLOR_PUNTO_HOVER := Color(1.0, 0.82, 0.30)
const COLOR_VIA := Color(0.85, 0.85, 0.88)
const COLOR_BASE := Color(0.16, 0.18, 0.23, 0.96)
const COLOR_TEXTO := Color(0.97, 0.97, 0.99)

# --- Generación de pasajeros (modelo por lotes, ver EstacionInterlock) ---
# Cada cuánto se genera un lote en una estación de afluencia base (1); una
# estación de afluencia 10 (Sants, Catalunya, Arc de Triomf, Sagrera) genera
# 10 veces más a menudo. El tamaño del lote es aleatorio dentro de un rango
# fijo -- no depende de la afluencia, que ya se refleja en la FRECUENCIA.
# (Bajado ~8x respecto a la primera versión: generaba demasiados pasajeros.)
const INTERVALO_BASE_GENERACION_SEG := 400.0
const TAMANO_LOTE_MIN := 2
const TAMANO_LOTE_MAX := 6

var datos: Dictionary = {}
var _num_vias: int = 1
var _nombre: String = ""
var _hover: bool = false
var _es_terminal: bool = false

var _interlock: EstacionInterlock = null
var _ui: EstacionUI = null
var _trenes_ref: Array = []          # referencia viva a la lista de trenes (para ETA)

var _afluencia: float = 1.0
var _acumulador_generacion_seg: float = 0.0
var _destinos_posibles: Array[String] = []   # ids de estacion alcanzables por alguna de nuestras lineas
var _pesos_destinos: Array[float] = []       # mismo indice que _destinos_posibles: afluencia de cada destino


func configurar(d: Dictionary, interlock: EstacionInterlock, ui: EstacionUI, trenes_ref: Array, es_terminal: bool = false) -> void:
	datos = d
	_num_vias = int(d.get("vias", 1))
	_nombre = str(d.get("nombre", d.get("id", "?")))
	_interlock = interlock
	_ui = ui
	_trenes_ref = trenes_ref
	_es_terminal = es_terminal
	_afluencia = maxf(float(d.get("afluencia", 1.0)), 0.1)
	_preparar_destinos_posibles()

	# Círculo de colisión para hover y clic.
	var col := CollisionShape2D.new()
	var forma := CircleShape2D.new()
	forma.radius = RADIO_HOVER
	col.shape = forma
	add_child(col)

	input_pickable = true
	mouse_entered.connect(_on_mouse_entered)
	mouse_exited.connect(_on_mouse_exited)
	input_event.connect(_on_input_event)
	queue_redraw()


## Precalcula (una sola vez) el conjunto de estaciones a las que un pasajero
## de aquí podría razonablemente ir: cualquier estación que comparta AL MENOS
## una de nuestras líneas (así un tren real de esa línea puede recogerlo, sin
## inventar trasbordos que el simulador no modela) junto con su peso
## (afluencia del DESTINO: los destinos más importantes son más probables).
func _preparar_destinos_posibles() -> void:
	var id_propio := str(datos.get("id", ""))
	var vistos: Dictionary = {}
	for linea in datos.get("lineas", []):
		for sid in Global.get_orden_estaciones_linea(str(linea)):
			var id_destino := str(sid)
			if id_destino == id_propio or vistos.has(id_destino):
				continue
			vistos[id_destino] = true
			_destinos_posibles.append(id_destino)
			_pesos_destinos.append(float(Global.get_estacion(id_destino).get("afluencia", 1.0)))


func _on_mouse_entered() -> void:
	_hover = true
	queue_redraw()


func _on_mouse_exited() -> void:
	_hover = false
	queue_redraw()


func _on_input_event(_vp: Node, event: InputEvent, _shape_idx: int) -> void:
	var mb := event as InputEventMouseButton
	if mb != null and mb.button_index == MOUSE_BUTTON_LEFT and mb.pressed:
		if _ui != null and _interlock != null:
			_ui.abrir(_interlock, _trenes_ref)


func _process(delta: float) -> void:
	# Tamaño de pantalla constante: contrarrestamos el zoom de la cámara.
	var cam := get_viewport().get_camera_2d()
	if cam != null:
		scale = Vector2.ONE / cam.zoom

	_actualizar_generacion_pasajeros(delta)


## Genera lotes de pasajeros al ritmo del reloj de juego (no del framerate):
## cada INTERVALO_BASE_GENERACION_SEG/(afluencia·demanda_del_momento) segundos
## de juego, un lote de tamaño aleatorio se suma al destino elegido (ver
## _elegir_destino_ponderado). La demanda del momento (Global.PERFIL_DEMANDA_DIA)
## es la misma para las 123 estaciones, así que apenas hay generación nada más
## abrir/justo antes de cerrar y dos puntas claras de hora punta, en vez de un
## flujo constante todo el día.
func _actualizar_generacion_pasajeros(delta: float) -> void:
	if _interlock == null or _destinos_posibles.is_empty() or Global.multiplicador_velocidad <= 0.0:
		return
	var seg_juego := delta * Global.FACTOR_BASE_TIEMPO * Global.multiplicador_velocidad
	_acumulador_generacion_seg += seg_juego
	var demanda := maxf(Global.multiplicador_demanda_pasajeros, 0.01)
	var intervalo := INTERVALO_BASE_GENERACION_SEG / (_afluencia * demanda)
	while _acumulador_generacion_seg >= intervalo:
		_acumulador_generacion_seg -= intervalo
		var destino := _elegir_destino_ponderado()
		if destino != "":
			_interlock.anadir_pasajeros(destino, randi_range(TAMANO_LOTE_MIN, TAMANO_LOTE_MAX))


## Elige un destino al azar, ponderado por la afluencia de cada uno (los
## destinos más importantes son proporcionalmente más probables).
func _elegir_destino_ponderado() -> String:
	var total := 0.0
	for peso in _pesos_destinos:
		total += peso
	if total <= 0.0:
		return ""
	var r := randf() * total
	var acumulado := 0.0
	for i in _destinos_posibles.size():
		acumulado += _pesos_destinos[i]
		if r <= acumulado:
			return _destinos_posibles[i]
	return _destinos_posibles[-1]


func _draw() -> void:
	var radio := RADIO_PUNTO_TERMINAL if _es_terminal else RADIO_PUNTO
	if not _hover:
		draw_circle(Vector2.ZERO, radio, COLOR_PUNTO)
		return

	# Vista al pasar el ratón: las N vías paralelas + punto resaltado + nombre.
	var ancho_total := float(_num_vias - 1) * SEPARACION_VIAS
	var x0 := -LARGO_VIA / 2.0
	var x1 := LARGO_VIA / 2.0
	var margen := 4.0

	draw_rect(Rect2(
		Vector2(x0 - margen, -ancho_total / 2.0 - margen),
		Vector2(LARGO_VIA + margen * 2.0, ancho_total + margen * 2.0)
	), COLOR_BASE, true)

	for i in _num_vias:
		var y := -ancho_total / 2.0 + float(i) * SEPARACION_VIAS
		draw_line(Vector2(x0, y), Vector2(x1, y), COLOR_VIA, 1.5)

	draw_circle(Vector2.ZERO, radio, COLOR_PUNTO_HOVER)

	var fuente := ThemeDB.fallback_font
	if fuente != null:
		var etiqueta := "%s · %d vías  (clic)" % [_nombre, _num_vias]
		draw_string(fuente, Vector2(x1 + 8.0, 4.0), etiqueta,
			HORIZONTAL_ALIGNMENT_LEFT, -1, 12, COLOR_TEXTO)
