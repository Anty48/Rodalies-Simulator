class_name FilaVia
extends Control
## ════════════════════════════════════════════════════════════════════════════
##  FilaVia.gd  —  Una fila de la "malla de vías" del panel de estación
## ════════════════════════════════════════════════════════════════════════════
##  Cada instancia dibuja UNA vía como una pista horizontal entre dos semáforos
##  cuadrados. La ORIENTACIÓN EN PANTALLA sigue la del mapa general: el lado A
##  (mayor latitud) queda a la DERECHA y el lado B a la IZQUIERDA — así que el
##  cuadrado izquierdo controla el semáforo de B y el derecho el de A:
##
##    [■ sem B] ──(azul si principal B, con ◄)──┼──(azul si principal A, ►)── [■ sem A]
##
##  - Clic en el tramo central (fuera de los cuadrados y del tren) → menú para
##    asignar ESTA vía como principal hacia el lado A o hacia el lado B.
##  - Clic en un cuadrado de semáforo → alterna verde/rojo de ese lado.
##  - Si hay un tren parado en la vía, se dibuja un rectángulo con el color de
##    su línea, el texto de la línea y un triángulo hacia el lado al que
##    partirá. Si además su semáforo de salida está en rojo, el propio
##    rectángulo se marca con "⇄" y un CLIC SOBRE ÉL invierte su sentido de
##    salida (sin botón flotante aparte: así la fila queda simétrica).
##
##  Es un widget AUTÓNOMO: lee el EstacionInterlock cada frame y se repinta
##  sola, igual que hace Semaforo.gd con su Bloque. El panel que la contiene
##  (EstacionUI) no necesita empujarle ningún estado.
## ════════════════════════════════════════════════════════════════════════════

const ALTO := 34.0
const LADO_SEM := 24.0
const ANCHO_PISTA := 170.0
const MARGEN := 4.0
const ANCHO_TOTAL := LADO_SEM * 2.0 + ANCHO_PISTA + MARGEN * 2.0

const TAM_FLECHA_VIA := 11.0
const TAM_FLECHA_TREN := 10.0
const ANCHO_TREN := 46.0

const C_VERDE := Color(0.30, 0.80, 0.42)
const C_ROJO := Color(0.88, 0.32, 0.32)
const C_AZUL := Color(0.35, 0.55, 0.95)
const C_BLANCO := Color(0.90, 0.90, 0.93)
const C_TEXTO := Color(0.95, 0.95, 0.98)
const C_TEXTO_OSCURO := Color(0.08, 0.08, 0.10)

var via: int = -1
var interlock: EstacionInterlock = null

var _sem_izq: Button    # controla el semáforo del lado B (izquierda de pantalla)
var _sem_der: Button    # controla el semáforo del lado A (derecha de pantalla)
var _menu: PopupMenu


func configurar(p_via: int, p_interlock: EstacionInterlock) -> void:
	via = p_via
	interlock = p_interlock
	custom_minimum_size = Vector2(ANCHO_TOTAL, ALTO)
	mouse_filter = Control.MOUSE_FILTER_STOP

	_sem_izq = _crear_boton_semaforo(Vector2(0.0, (ALTO - LADO_SEM) / 2.0))
	_sem_izq.pressed.connect(func() -> void: interlock.alternar_semaforo(via, "B"))

	_sem_der = _crear_boton_semaforo(Vector2(ANCHO_TOTAL - LADO_SEM, (ALTO - LADO_SEM) / 2.0))
	_sem_der.pressed.connect(func() -> void: interlock.alternar_semaforo(via, "A"))

	_menu = PopupMenu.new()
	_menu.id_pressed.connect(_on_menu_via)
	add_child(_menu)


func _crear_boton_semaforo(pos: Vector2) -> Button:
	var b := Button.new()
	b.position = pos
	b.custom_minimum_size = Vector2(LADO_SEM, LADO_SEM)
	b.size = b.custom_minimum_size
	var sb := StyleBoxFlat.new()
	sb.bg_color = C_VERDE
	sb.set_corner_radius_all(4)
	b.add_theme_stylebox_override("normal", sb)
	b.add_theme_stylebox_override("hover", sb)
	b.add_theme_stylebox_override("pressed", sb)
	b.add_theme_stylebox_override("focus", sb)
	add_child(b)
	return b


func _process(_delta: float) -> void:
	if interlock == null:
		return
	var sb_izq := _sem_izq.get_theme_stylebox("normal") as StyleBoxFlat
	sb_izq.bg_color = C_VERDE if interlock.semaforo_verde(via, "B") else C_ROJO
	var sb_der := _sem_der.get_theme_stylebox("normal") as StyleBoxFlat
	sb_der.bg_color = C_VERDE if interlock.semaforo_verde(via, "A") else C_ROJO
	queue_redraw()


## Rectángulo del tren estacionado, en coordenadas locales de la fila. Se usa
## tanto para dibujarlo como para saber si un clic ha caído sobre él.
func _rect_tren() -> Rect2:
	var y := ALTO / 2.0
	var x_centro := ANCHO_TOTAL / 2.0
	var alto_tren := ALTO - 8.0
	return Rect2(x_centro - ANCHO_TREN / 2.0, y - alto_tren / 2.0, ANCHO_TREN, alto_tren)


func _gui_input(event: InputEvent) -> void:
	var mb := event as InputEventMouseButton
	if mb == null or mb.button_index != MOUSE_BUTTON_LEFT or not mb.pressed:
		return
	# Los cuadrados de semáforo son Button hijos: si el clic cae ahí, ya lo
	# habrán consumido ellos y este código ni se ejecuta.
	var tren := interlock.tren_en(via) as Tren
	if tren != null and _rect_tren().has_point(mb.position):
		# Clic directamente sobre el tren parado: invierte su sentido de
		# salida. Es la única forma de invertir — nada de botón flotante.
		tren.invertir_sentido_salida()
		return

	# Clic en el resto del tramo: menú para asignar esta vía como principal.
	_menu.clear()
	_menu.add_item("← Vía principal hacia %s" % _texto_corto(interlock.terminales_b), 1)
	_menu.add_item("Vía principal hacia %s →" % _texto_corto(interlock.terminales_a), 0)
	_menu.position = Vector2i(get_viewport().get_mouse_position())
	_menu.reset_size()
	_menu.popup()


func _on_menu_via(id: int) -> void:
	if id == 0:
		interlock.asignar_principal(via, "A")
	elif id == 1:
		interlock.asignar_principal(via, "B")


func _draw() -> void:
	var y := ALTO / 2.0
	var x_izq := LADO_SEM + MARGEN
	var x_der := ANCHO_TOTAL - LADO_SEM - MARGEN
	var x_centro := ANCHO_TOTAL / 2.0

	# El lado A se dibuja a la DERECHA y el lado B a la IZQUIERDA (ver cabecera).
	var principal_a := interlock.es_principal(via, "A")
	var principal_b := interlock.es_principal(via, "B")

	draw_line(Vector2(x_izq, y), Vector2(x_centro, y), C_AZUL if principal_b else C_BLANCO, 3.0 if principal_b else 2.0)
	if principal_b:
		_triangulo(Vector2(x_izq + 5.0, y), false, TAM_FLECHA_VIA, C_AZUL)

	draw_line(Vector2(x_centro, y), Vector2(x_der, y), C_AZUL if principal_a else C_BLANCO, 3.0 if principal_a else 2.0)
	if principal_a:
		_triangulo(Vector2(x_der - 5.0, y), true, TAM_FLECHA_VIA, C_AZUL)

	var fuente := ThemeDB.fallback_font
	if fuente != null:
		draw_string(fuente, Vector2(x_izq, y - 8.0), "Vía %d" % (via + 1),
			HORIZONTAL_ALIGNMENT_LEFT, -1, 10, Color(0.6, 0.62, 0.68))

	var tren := interlock.tren_en(via) as Tren
	if tren == null:
		return

	var rect := _rect_tren()
	draw_rect(rect, tren.color, true)
	var invertible := not interlock.semaforo_verde(via, tren.direccion_salida())
	draw_rect(rect, Color(1.0, 0.9, 0.3, 0.95) if invertible else Color(0, 0, 0, 0.7), false, 2.0 if invertible else 1.5)

	if fuente != null:
		var brillo := (tren.color.r * 0.299 + tren.color.g * 0.587 + tren.color.b * 0.114)
		var color_txt := C_TEXTO_OSCURO if brillo > 0.55 else C_TEXTO
		var texto := tren.id_linea + (" ⇄" if invertible else "")
		if tren.retraso_seg() > 60.0:
			texto += " +%dm" % int(tren.retraso_seg() / 60.0)
		draw_string(fuente, rect.position + Vector2(3.0, rect.size.y / 2.0 + 4.0), texto,
			HORIZONTAL_ALIGNMENT_LEFT, rect.size.x - 3.0, 11, color_txt)

	# Triángulo de sentido: hacia qué lado partirá este tren al arrancar
	# (lado A = derecha, lado B = izquierda, igual que las vías principales).
	var hacia_derecha := tren.direccion_salida() == "A"
	var punta := Vector2(rect.position.x + rect.size.x + 6.0, y) if hacia_derecha else Vector2(rect.position.x - 6.0, y)
	_triangulo(punta, hacia_derecha, TAM_FLECHA_TREN, Color(0.95, 0.9, 0.4))


## Dibuja un triángulo apuntando a la derecha (hacia_derecha = true) o a la
## izquierda, con la punta en "punta".
func _triangulo(punta: Vector2, hacia_derecha: bool, tam: float, color: Color) -> void:
	var signo := 1.0 if hacia_derecha else -1.0
	var p1 := punta
	var p2 := punta - Vector2(signo * tam, -tam * 0.7)
	var p3 := punta - Vector2(signo * tam, tam * 0.7)
	draw_colored_polygon(PackedVector2Array([p1, p2, p3]), color)


func _texto_corto(t: String) -> String:
	if t.length() <= 28:
		return t
	return t.substr(0, 26) + "…"
