class_name CamaraMapa
extends Camera2D
## ════════════════════════════════════════════════════════════════════════════
##  CamaraMapa.gd  —  Cámara con desplazamiento y zoom (hacia el cursor)
## ════════════════════════════════════════════════════════════════════════════
##  - Arrastrar con el botón IZQUIERDO  ->  desplazar (pan).
##  - Rueda del ratón                    ->  acercar / alejar, hacia el cursor.
##
##  Nota Godot 4: en `zoom`, un valor MAYOR significa MÁS cerca.
## ════════════════════════════════════════════════════════════════════════════

const ZOOM_MIN := 0.06     # muy alejado (se ve toda Catalunya pequeñita)
const ZOOM_MAX := 50.0     # muy cerca (se ven trenes y vías enormes)
const ZOOM_PASO := 1.15    # cuánto cambia el zoom por cada "clic" de rueda
const DURACION_IR_A := 0.6 # segundos que tarda el tween de "ir a" (ver TrenesPanel "Ir al tren")

## Deslizamiento al soltar el arrastre (efecto "hielo": antes el mapa se paraba
## en seco en el mismo fotograma en que se soltaba el botón). FRICCION más
## alto frena antes; VELOCIDAD_MINIMA es el umbral (unidades de mundo/seg) por
## debajo del cual se considera parado y se corta el deslizamiento.
const FRICCION_DESLIZAMIENTO := 4.0
const VELOCIDAD_MINIMA_DESLIZAMIENTO := 4.0

## Cuánto margen extra (factor sobre el tamaño real de la pantalla) se añade
## al rectángulo de "culling" (ver rect_visible) para que los elementos no
## aparezcan/desaparezcan de golpe justo al borde de la pantalla.
const MARGEN_CULLING := 1.25

## Estado COMPARTIDO de la cámara, recalculado una vez por fotograma (ver
## _actualizar_estado_compartido) para que cualquier nodo (Semaforo.gd es el
## motivo: cientos de ellos en el mapa) pueda consultar "¿estoy en pantalla
## ahora mismo?" y "¿a qué escala tengo que dibujarme?" sin buscar la cámara
## ni repetir el cálculo cada uno por su cuenta — antes cada nodo llamaba a
## get_viewport().get_camera_2d() y recalculaba esto en su propio _process(),
## Y ADEMÁS lo hacía aunque estuviera fuera de la pantalla (la visibilidad de
## la CAPA es un solo booleano para las 8 líneas: al hacer zoom en cualquier
## punto del mapa, TODOS los semáforos de todas partes se ponían a trabajar
## cada fotograma, no solo el puñado realmente visible).
static var rect_visible := Rect2()
static var escala_pantalla := Vector2.ONE

var _arrastrando := false
var _velocidad_deslizamiento := Vector2.ZERO   # unidades de mundo/seg, impulso que queda al soltar
var _tween_ir_a: Tween = null


func _unhandled_input(event: InputEvent) -> void:
	var boton := event as InputEventMouseButton
	if boton != null:
		if boton.button_index == MOUSE_BUTTON_WHEEL_UP and boton.pressed:
			_aplicar_zoom(ZOOM_PASO)
		elif boton.button_index == MOUSE_BUTTON_WHEEL_DOWN and boton.pressed:
			_aplicar_zoom(1.0 / ZOOM_PASO)
		elif boton.button_index == MOUSE_BUTTON_LEFT:
			_arrastrando = boton.pressed
		return

	var mov := event as InputEventMouseMotion
	if mov != null and _arrastrando:
		position -= mov.relative / zoom
		# Guardamos la velocidad instantánea del ratón (Godot ya la da en
		# píxeles de pantalla/seg) convertida a unidades de mundo: es el
		# impulso con el que seguirá deslizando la cámara si se suelta aquí
		# mismo, en vez de frenar en seco (ver _process).
		_velocidad_deslizamiento = -mov.velocity / zoom


func _process(delta: float) -> void:
	_actualizar_estado_compartido()

	if _arrastrando or _velocidad_deslizamiento.length() < VELOCIDAD_MINIMA_DESLIZAMIENTO:
		_velocidad_deslizamiento = Vector2.ZERO
		return
	position += _velocidad_deslizamiento * delta
	# Fricción exponencial (no lineal): se nota como un frenado progresivo de
	# verdad, deslizando cada vez más despacio, en vez de un tramo a velocidad
	# constante que se corta de golpe.
	_velocidad_deslizamiento = _velocidad_deslizamiento.lerp(Vector2.ZERO, clampf(FRICCION_DESLIZAMIENTO * delta, 0.0, 1.0))


func _actualizar_estado_compartido() -> void:
	escala_pantalla = Vector2.ONE / zoom
	var tam := get_viewport_rect().size * escala_pantalla * MARGEN_CULLING
	var centro := get_screen_center_position()
	rect_visible = Rect2(centro - tam / 2.0, tam)


## Cambia el zoom manteniendo fijo el punto del mundo que está bajo el cursor.
## (Así "te acercas a donde miras", en vez de al centro de la pantalla.)
func _aplicar_zoom(factor: float) -> void:
	var z0 := zoom
	var nuevo := clampf(zoom.x * factor, ZOOM_MIN, ZOOM_MAX)
	var z1 := Vector2(nuevo, nuevo)

	# Posición del ratón en pantalla y centro de la pantalla.
	var raton := get_viewport().get_mouse_position()
	var centro := get_viewport_rect().size / 2.0

	# Corrección para que el punto bajo el cursor no se mueva al hacer zoom.
	position += (raton - centro) * (Vector2.ONE / z0 - Vector2.ONE / z1)
	zoom = z1


## "Ir al tren" (ver TrenesPanel): desplaza la cámara suavemente hasta centrarla
## en `destino` (posición del mundo), sin tocar el zoom. Si ya había un viaje en
## marcha (doble clic en "Ir" antes de que acabe), lo cancelamos y empezamos el
## nuevo desde la posición actual, para no dejar dos tweens peleándose por `position`.
func ir_a(destino: Vector2) -> void:
	_velocidad_deslizamiento = Vector2.ZERO   # que no compita con el tween por `position`
	if _tween_ir_a != null and _tween_ir_a.is_valid():
		_tween_ir_a.kill()
	_tween_ir_a = create_tween()
	_tween_ir_a.set_trans(Tween.TRANS_CUBIC).set_ease(Tween.EASE_OUT)
	_tween_ir_a.tween_property(self, "position", destino, DURACION_IR_A)
