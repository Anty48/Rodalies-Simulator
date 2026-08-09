class_name EstacionUI
extends CanvasLayer
## ════════════════════════════════════════════════════════════════════════════
##  EstacionUI.gd  —  Panel de enclavamiento de una estación (3 columnas)
## ════════════════════════════════════════════════════════════════════════════
##  Hay UN solo panel compartido por todo el mapa. Al hacer clic en una estación,
##  el nodo Estacion llama a abrir(interlock, trenes) y el panel se reconstruye
##  para ESA estación.
##
##  Estructura de tres columnas verticales (una línea NO tiene norte/sur: tiene
##  dos terminales, así que las llamamos simplemente "lado A" y "lado B"):
##
##    ┌───────────────┬──────────────────────────┬───────────────┐
##    │  Destinos      │   Malla de vías (una      │   Destinos     │
##    │  lado A        │   fila FilaVia por vía,   │   lado B       │
##    │  (nombre real  │   con sus semáforos y     │  (nombre real  │
##    │   del destino) │   el tren si lo hay)      │   del destino) │
##    └───────────────┴──────────────────────────┴───────────────┘
##
##  Cada FilaVia es un widget autónomo (ver FilaVia.gd): se dibuja y reacciona
##  a clics sola, leyendo el EstacionInterlock cada frame. Este panel solo la
##  crea y la coloca; no gestiona su estado.
## ════════════════════════════════════════════════════════════════════════════

const ANCHO_COL_TERMINAL := 150.0

## Alto (en píxeles) que ocupa todo lo del panel QUE NO es la malla de vías:
## cabecera, próximo tren, separador, título "Vías", pie de ayuda y los
## márgenes/separaciones del panel. Se resta del alto real de la ventana para
## saber cuánto le queda de sitio a la malla antes de que el panel entero se
## salga de la pantalla. Es una estimación (no medimos los nodos reales
## porque aún no existen en este punto de _reconstruir()); mejor quedarse
## corto y hacer scroll un poco antes de lo estrictamente necesario que
## desbordar la ventana.
const ALTO_CROMO_PANEL := 200.0
const ALTO_MIN_MALLA_VIAS := 150.0   # nunca reducir el hueco de la malla por debajo de esto

var _interlock: EstacionInterlock = null
var _trenes: Array = []
var _panel: PanelContainer
var _titulo: Label
var _eta: Label
var _lbl_pax_a: Label = null
var _lbl_pax_b: Label = null


func _ready() -> void:
	layer = 10
	_panel = PanelContainer.new()
	_panel.position = Vector2(24, 90)
	_panel.visible = false

	# Fondo casi opaco: con el panel semitransparente las vías del mapa que
	# pasan por detrás se mezclaban con los nombres de las estaciones y
	# dificultaban la lectura.
	var sb_fondo := StyleBoxFlat.new()
	sb_fondo.bg_color = Color(0.08, 0.09, 0.11, 0.94)
	sb_fondo.set_corner_radius_all(8)
	sb_fondo.content_margin_left = 14.0
	sb_fondo.content_margin_right = 14.0
	sb_fondo.content_margin_top = 12.0
	sb_fondo.content_margin_bottom = 12.0
	_panel.add_theme_stylebox_override("panel", sb_fondo)

	add_child(_panel)


func abrir(interlock: EstacionInterlock, trenes: Array) -> void:
	if interlock == null:
		return
	_interlock = interlock
	_trenes = trenes
	_reconstruir()
	_panel.visible = true


func cerrar() -> void:
	_panel.visible = false


func _process(_delta: float) -> void:
	if _interlock == null or not _panel.visible:
		return
	_actualizar_eta()
	_actualizar_pasajeros()


# ─────────────────────────────────────────────────────────────────────────────
#  CONSTRUCCIÓN DEL PANEL
# ─────────────────────────────────────────────────────────────────────────────

func _reconstruir() -> void:
	for c in _panel.get_children():
		_panel.remove_child(c)
		c.queue_free()

	var vb := VBoxContainer.new()
	vb.add_theme_constant_override("separation", 8)
	_panel.add_child(vb)

	# Cabecera: nombre + botón cerrar.
	var cab := HBoxContainer.new()
	vb.add_child(cab)
	_titulo = Label.new()
	_titulo.text = "%s · %d vías" % [_interlock.nombre, _interlock.num_vias]
	_titulo.add_theme_font_size_override("font_size", 18)
	cab.add_child(_titulo)
	var sep := Control.new()
	sep.custom_minimum_size = Vector2(20, 0)
	sep.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	cab.add_child(sep)
	var btn_cerrar := Button.new()
	btn_cerrar.text = "✕"
	btn_cerrar.pressed.connect(cerrar)
	cab.add_child(btn_cerrar)

	# Próximo tren.
	_eta = Label.new()
	_eta.text = "Próximo tren: —"
	vb.add_child(_eta)

	vb.add_child(HSeparator.new())

	# Las tres columnas. La orientación sigue la del mapa general: el lado A
	# (mayor latitud) queda a la DERECHA y el lado B a la IZQUIERDA — así el
	# ojo lee la columna hacia el lado de la pantalla donde de verdad se
	# extienden esas vías, en vez de al revés.
	var columnas := HBoxContainer.new()
	columnas.add_theme_constant_override("separation", 12)
	vb.add_child(columnas)

	var pax := _calcular_pasajeros_por_lado()
	var pax_a: int = pax["A"]
	var pax_b: int = pax["B"]

	var col_b := _columna_terminal("◄ Lado B", _interlock.terminales_b, HORIZONTAL_ALIGNMENT_RIGHT, pax_b)
	_lbl_pax_b = col_b.get_child(2) as Label
	columnas.add_child(col_b)
	columnas.add_child(VSeparator.new())

	var col_vias := VBoxContainer.new()
	col_vias.add_theme_constant_override("separation", 4)
	var titulo_vias := Label.new()
	titulo_vias.text = "Vías"
	titulo_vias.horizontal_alignment = HORIZONTAL_ALIGNMENT_CENTER
	titulo_vias.add_theme_font_size_override("font_size", 12)
	col_vias.add_child(titulo_vias)

	# Estaciones con muchas vías (p. ej. Montcada Bifurcació con 15, o Estació
	# de França con 12) desbordarían la pantalla si dibujáramos todas las
	# filas sueltas. Regla GENERAL (no solo para esas dos): la malla nunca
	# ocupa más alto del que de verdad queda libre en la ventana actual: a
	# partir de ahí, ScrollContainer se encarga de la barra vertical.
	var alto_ventana := get_viewport().get_visible_rect().size.y
	var alto_disponible := maxf(ALTO_MIN_MALLA_VIAS, alto_ventana - _panel.position.y - ALTO_CROMO_PANEL)
	var alto_contenido := _interlock.num_vias * (FilaVia.ALTO + 4.0)
	var scroll := ScrollContainer.new()
	scroll.horizontal_scroll_mode = ScrollContainer.SCROLL_MODE_DISABLED
	scroll.custom_minimum_size = Vector2(0, minf(alto_disponible, alto_contenido))
	var filas := VBoxContainer.new()
	filas.add_theme_constant_override("separation", 4)
	# Estaciones con corredores segregados (p. ej. El Clot: R1 en unas vías,
	# R2/R2_NORD en otras) muestran una etiqueta separando cada grupo, para que
	# se entienda por qué hay más de una vía "principal" del mismo lado a la
	# vez — cada corredor tiene la suya, son físicamente independientes.
	var multi_corredor := _interlock.num_corredores() > 1
	for c in _interlock.num_corredores():
		if multi_corredor:
			var lbl_corredor := Label.new()
			lbl_corredor.text = "── %s ──" % _interlock.etiqueta_corredor(c)
			lbl_corredor.horizontal_alignment = HORIZONTAL_ALIGNMENT_CENTER
			lbl_corredor.add_theme_font_size_override("font_size", 10)
			lbl_corredor.modulate = Color(0.6, 0.62, 0.68)
			filas.add_child(lbl_corredor)
		for v in _interlock.vias_de_corredor(c):
			var fila := FilaVia.new()
			filas.add_child(fila)
			fila.configurar(int(v), _interlock)
	scroll.add_child(filas)
	col_vias.add_child(scroll)
	columnas.add_child(col_vias)

	columnas.add_child(VSeparator.new())
	var col_a := _columna_terminal("Lado A ►", _interlock.terminales_a, HORIZONTAL_ALIGNMENT_LEFT, pax_a)
	_lbl_pax_a = col_a.get_child(2) as Label
	columnas.add_child(col_a)

	# Pie de ayuda.
	var ayuda := Label.new()
	ayuda.text = "Cuadros = semáforo interno de cada lado (clic alterna verde/rojo).\nCentro de la vía = asignarla como principal. Con un tren parado (marcado ⇄ si su semáforo está en rojo), clic sobre el propio tren invierte su sentido de salida."
	ayuda.add_theme_font_size_override("font_size", 11)
	ayuda.modulate = Color(0.7, 0.72, 0.78)
	vb.add_child(ayuda)

	_actualizar_eta()


## Columna con el nombre real de las estaciones terminal de un lado (nunca
## "Norte"/"Sur": el texto que llega ya es el nombre de la parada), con el
## número de pasajeros esperando para ESE sentido justo debajo.
func _columna_terminal(titulo: String, texto: String, alineacion: HorizontalAlignment, pasajeros: int = 0) -> VBoxContainer:
	var col := VBoxContainer.new()
	col.custom_minimum_size = Vector2(ANCHO_COL_TERMINAL, 0)

	var lbl_titulo := Label.new()
	lbl_titulo.text = titulo
	lbl_titulo.horizontal_alignment = alineacion
	lbl_titulo.add_theme_font_size_override("font_size", 12)
	lbl_titulo.modulate = Color(0.7, 0.72, 0.78)
	col.add_child(lbl_titulo)

	var lbl_texto := Label.new()
	lbl_texto.text = texto if texto != "" else "—"
	lbl_texto.horizontal_alignment = alineacion
	lbl_texto.autowrap_mode = TextServer.AUTOWRAP_WORD_SMART
	lbl_texto.add_theme_font_size_override("font_size", 13)
	lbl_texto.custom_minimum_size = Vector2(ANCHO_COL_TERMINAL, 0)
	col.add_child(lbl_texto)

	var lbl_pax := Label.new()
	lbl_pax.text = "%d esperando" % pasajeros
	lbl_pax.horizontal_alignment = alineacion
	lbl_pax.add_theme_font_size_override("font_size", 11)
	lbl_pax.modulate = Color(0.95, 0.8, 0.35) if pasajeros > 0 else Color(0.55, 0.57, 0.62)
	col.add_child(lbl_pax)

	return col


## ¿En qué lado (A/B) de esta estación cae `id_destino`, por alguna de sus
## propias líneas? Mismo criterio que MapaCatalunya._crear_linea (terminal de
## mayor latitud = lado A), recalculado aquí a partir de datos globales (no
## depende de tener la tabla cacheada de MapaCatalunya) porque solo hace
## falta al construir este panel, no en cada fotograma.
func _lado_de_destino(id_destino: String) -> String:
	for id_linea in Global.get_estacion(_interlock.id).get("lineas", []):
		var ids := Global.get_orden_estaciones_linea(str(id_linea))
		var idx_propio := ids.find(_interlock.id)
		var idx_destino := ids.find(id_destino)
		if idx_propio < 0 or idx_destino < 0 or idx_propio == idx_destino:
			continue
		var terminales: Array = Global.get_linea(str(id_linea)).get("terminales", [])
		var e0 := str(terminales[0]) if terminales.size() > 0 else str(ids[0])
		var e1 := str(terminales[1]) if terminales.size() > 1 else str(ids[ids.size() - 1])
		var lat0 := float(Global.get_estacion(e0).get("lat", 0.0))
		var lat1 := float(Global.get_estacion(e1).get("lat", 0.0))
		var id_lado_a := e1 if lat1 >= lat0 else e0
		var lado_a_es_fin := ids.find(id_lado_a) == ids.size() - 1
		var hacia_indices_crecientes := idx_destino > idx_propio
		if hacia_indices_crecientes:
			return "A" if lado_a_es_fin else "B"
		return "B" if lado_a_es_fin else "A"
	return ""


## Suma _interlock.pasajeros_por_destino agrupado por lado ("A"/"B"; lo que no
## se pueda resolver a ningún lado -- destino fuera de nuestras líneas -- no
## cuenta en ninguno de los dos, no debería pasar en la práctica).
func _calcular_pasajeros_por_lado() -> Dictionary:
	var total := {"A": 0, "B": 0}
	for id_destino in _interlock.pasajeros_por_destino.keys():
		var cantidad := _interlock.pasajeros_para(str(id_destino))
		var lado := _lado_de_destino(str(id_destino))
		if lado == "A" or lado == "B":
			total[lado] = int(total[lado]) + cantidad
	return total


## Refresca (sin reconstruir el panel entero) los dos contadores de pasajeros
## esperando, igual que _actualizar_eta hace con el próximo tren -- se llama
## cada fotograma mientras el panel está abierto, así se ve crecer/vaciar en
## vivo sin tener que cerrar y volver a abrir el panel.
func _actualizar_pasajeros() -> void:
	if _lbl_pax_a == null or _lbl_pax_b == null:
		return
	var pax := _calcular_pasajeros_por_lado()
	var pax_a: int = pax["A"]
	var pax_b: int = pax["B"]
	_lbl_pax_a.text = "%d esperando" % pax_a
	_lbl_pax_a.modulate = Color(0.95, 0.8, 0.35) if pax_a > 0 else Color(0.55, 0.57, 0.62)
	_lbl_pax_b.text = "%d esperando" % pax_b
	_lbl_pax_b.modulate = Color(0.95, 0.8, 0.35) if pax_b > 0 else Color(0.55, 0.57, 0.62)


func _actualizar_eta() -> void:
	var mejor := -1.0
	for t in _trenes:
		var tren := t as Tren
		if tren == null:
			continue
		if tren.proxima_estacion_id() == _interlock.id:
			var e := tren.eta_segundos_juego()
			if e >= 0.0 and (mejor < 0.0 or e < mejor):
				mejor = e
	if mejor < 0.0:
		_eta.text = "Próximo tren: —"
	else:
		var m := int(mejor / 60.0)
		var s := int(mejor) % 60
		_eta.text = "Próximo tren: %d:%02d (tiempo de juego)" % [m, s]
