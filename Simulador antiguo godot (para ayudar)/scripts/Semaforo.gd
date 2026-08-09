class_name Semaforo
extends Area2D
## ════════════════════════════════════════════════════════════════════════════
##  Semaforo.gd  —  Señal de tramo: un TRIÁNGULO que apunta en el sentido de la vía
## ════════════════════════════════════════════════════════════════════════════
##  Verde = bloque libre. Rojo = ocupado o cerrado a mano. Aro amarillo = manual.
##  Clic izquierdo -> menú: Automático / Manual: Verde / Manual: Rojo.
##  Se coloca justo al lado de su vía (la del sentido que protege) y apunta hacia
##  donde circulan los trenes de ese sentido.
##
##  MODO ENCLAVAMIENTO (ver configurar_enclavamiento — vía única, checklist
##  #3): en vez de un Bloque.modo manual independiente, el clic alterna
##  DIRECTAMENTE el semáforo INTERNO de la estación de origen (el mismo
##  booleano que ya controla el cuadrado de FilaVia en el panel interior) —
##  así el semáforo del mapa y el de la interfaz son, en la práctica, el mismo
##  estado visto desde dos sitios distintos.
## ════════════════════════════════════════════════════════════════════════════

const T := 6.0   # tamaño del triángulo, en píxeles de pantalla

var bloque: Bloque
var _popup: PopupMenu = null

var _interlock: EstacionInterlock = null   # solo en modo enclavamiento
var _lado: String = ""
var _id_linea: String = ""
var _modo_enclavamiento: bool = false


func configurar(b: Bloque, pos: Vector2, direccion: Vector2) -> void:
	bloque = b
	position = pos
	# Orientamos el nodo hacia el sentido de la vía; el triángulo se dibuja en
	# local apuntando a +X, así que rota con nosotros.
	if direccion.length() > 0.001:
		rotation = direccion.angle()

	var col := CollisionShape2D.new()
	var forma := CircleShape2D.new()
	forma.radius = T * 1.6
	col.shape = forma
	add_child(col)

	input_pickable = true
	# Solo nos interesa la detección del ratón (picking), no la superposición
	# física entre Areas (igual que Tren.gd) — con cientos de semáforos en el
	# mapa (uno o varios por tramo y sentido, en las 8 líneas, más los nuevos
	# de vía única), dejar monitoring/monitorable activos por defecto obligaba
	# al motor a comprobar solapes de TODOS contra TODOS cada fotograma físico
	# sin que nadie escuchara esa señal — el origen real del lag al acercarse.
	monitoring = false
	monitorable = false
	input_event.connect(_on_input_event)


## Semáforo de salida de un tramo de VÍA ÚNICA: reusa toda la parte común
## (posición/rotación/colisión) de configurar(), pero el clic y el color pasan
## a depender del semáforo interno de `interlock` para `lado` (vía principal
## de ESE lado, resuelta en el momento — puede cambiar si la jugadora
## reasigna la vía principal desde el panel interior).
func configurar_enclavamiento(b: Bloque, interlock: EstacionInterlock, lado: String, id_linea: String, pos: Vector2, direccion: Vector2) -> void:
	configurar(b, pos, direccion)
	_interlock = interlock
	_lado = lado
	_id_linea = id_linea
	_modo_enclavamiento = true


func _process(_delta: float) -> void:
	# Dos filtros antes de hacer nada: (1) el LOD de MapaCatalunya oculta toda
	# la capa hasta cierto zoom (ver UMBRAL_SEMAFOROS) — pero eso es un solo
	# booleano para las 8 líneas, así que en cuanto se hace zoom en CUALQUIER
	# punto del mapa la capa entera pasa a "visible"; (2) por eso además
	# comprobamos si ESTE semáforo en concreto está dentro de lo que la cámara
	# ve AHORA MISMO (CamaraMapa.rect_visible, actualizado una vez por
	# fotograma) — con cientos de semáforos en el mapa, sin este segundo
	# filtro TODOS recalculaban su escala y pedían un redibujado cada
	# fotograma aunque solo un puñado estuviera realmente en pantalla, lo que
	# causaba caídas de FPS notables al acercarse en zonas con mucha vía.
	if not is_visible_in_tree() or not CamaraMapa.rect_visible.has_point(global_position):
		return
	scale = CamaraMapa.escala_pantalla
	queue_redraw()


func _on_input_event(_viewport: Node, event: InputEvent, _shape: int) -> void:
	var mb := event as InputEventMouseButton
	if mb == null or mb.button_index != MOUSE_BUTTON_LEFT or not mb.pressed:
		return
	if _modo_enclavamiento:
		if _popup == null:
			_crear_popup_enclavamiento()
	elif _popup == null:
		_crear_popup()
	_popup.position = Vector2i(get_viewport().get_mouse_position())
	_popup.popup()


func _crear_popup() -> void:
	_popup = PopupMenu.new()
	_popup.add_item("Automático", 0)
	_popup.add_item("Manual: Verde", 1)
	_popup.add_item("Manual: Rojo", 2)
	_popup.id_pressed.connect(_on_opcion)
	add_child(_popup)


func _crear_popup_enclavamiento() -> void:
	_popup = PopupMenu.new()
	_popup.add_item("Alternar semáforo interno de salida", 0)
	_popup.id_pressed.connect(_on_opcion_enclavamiento)
	add_child(_popup)


func _on_opcion(id: int) -> void:
	if id == 0:
		bloque.modo = Bloque.Modo.AUTO
	elif id == 1:
		bloque.modo = Bloque.Modo.MANUAL_VERDE
	elif id == 2:
		bloque.modo = Bloque.Modo.MANUAL_ROJO


func _on_opcion_enclavamiento(_id: int) -> void:
	_interlock.alternar_semaforo(_interlock.via_principal(_lado, _id_linea), _lado)


func _draw() -> void:
	var color := Color(0.90, 0.20, 0.20) if _en_rojo_efectivo() else Color(0.20, 0.85, 0.35)
	var p1 := Vector2(T, 0.0)              # punta (apunta al sentido de marcha)
	var p2 := Vector2(-T * 0.7, T * 0.7)
	var p3 := Vector2(-T * 0.7, -T * 0.7)
	draw_colored_polygon(PackedVector2Array([p1, p2, p3]), color)
	if not _modo_enclavamiento and bloque.es_manual():
		draw_polyline(PackedVector2Array([p1, p2, p3, p1]), Color(1.0, 0.9, 0.3, 0.95), 1.0)


## En modo enclavamiento, además de lo que ya diga el Bloque (tránsito físico
## + testigo de vía única, ver Bloque.en_rojo), el semáforo se apaga si el
## semáforo INTERNO de la vía principal de este lado está en rojo — el mismo
## estado que ya gestiona FilaVia en el panel interior de la estación.
func _en_rojo_efectivo() -> bool:
	if _modo_enclavamiento:
		var via := _interlock.via_principal(_lado, _id_linea)
		if not _interlock.semaforo_verde(via, _lado):
			return true
	return bloque.en_rojo()
