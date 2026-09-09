class_name TrenesPanel
extends CanvasLayer
## ════════════════════════════════════════════════════════════════════════════
##  TrenesPanel.gd  —  Panel de depuración: lista de todos los trenes en marcha
## ════════════════════════════════════════════════════════════════════════════
##  Pensado para depurar el motor de horarios y la señalización sin tener que
##  ir haciendo capturas de pantalla: para cada tren, su ID, línea, ubicación
##  actual (estación o cantón), destino final y retraso — con filtro por
##  línea/retraso y orden por línea o por retraso. Se abre/cierra desde el
##  botón "Trenes" del HUD (ver HUD.gd).
##
##  RENDIMIENTO: con ~100 trenes en marcha, reconstruir la lista entera
##  (borrar y crear de nuevo cada Label/Control) 60 veces por segundo generaba
##  un lag muy severo. Dos cambios lo evitan:
##   1) Refresco LIMITADO a INTERVALO_REFRESCO_SEG (1 vez por segundo REAL),
##      no en cada _process(): la lista es para leerla con calma, no necesita
##      más frecuencia que esa.
##   2) POOL DE FILAS: las filas (envoltorio + labels) se crean una única vez
##      y se REUTILIZAN entre refrescos — solo se actualiza el TEXTO/COLOR de
##      los Labels ya existentes; las filas que sobran de un refresco a otro
##      simplemente se ocultan (`visible = false`), nunca se destruyen. El pool
##      solo crece la primera vez que hace falta más filas de las que ya había
##      (como mucho, una vez por cada tren nuevo que se vea alguna vez en la
##      lista) — a partir de ahí, cero asignaciones de nodos por refresco.
## ════════════════════════════════════════════════════════════════════════════

const MARGEN_PANTALLA := 40.0          # el panel ocupa toda la pantalla menos este margen
const INTERVALO_REFRESCO_SEG := 1.0    # segundos REALES entre refrescos de la lista

# Anchos de columna compartidos por la cabecera y cada fila, para que quede
# todo alineado como una tabla real aunque estén hechas de Labels sueltos
# (no usamos GridContainer: así podemos pintar cada fila entera, incluida su
# franja de cebra, con un único PanelContainer reutilizable).
const ANCHO_COL_ID := 64.0
const ANCHO_COL_LINEA := 84.0
const ANCHO_COL_RETRASO := 100.0
const ANCHO_COL_IR := 90.0

# Umbrales de retraso (segundos) para el color del texto: por encima de cada
# uno se considera un escalón peor (puntual -> atención -> crítico).
const RETRASO_ATENCION_SEG := 60.0
const RETRASO_CRITICO_SEG := 300.0

const C_FILA_PAR := Color(1, 1, 1, 0.028)
const C_FILA_IMPAR := Color(1, 1, 1, 0.0)
const C_CABECERA := Color(0.7, 0.72, 0.78)
const C_PUNTUAL := Color(0.75, 0.78, 0.8)
const C_ATENCION := Color(1.0, 0.72, 0.35)
const C_CRITICO := Color(1.0, 0.42, 0.4)

var _trenes: Array = []
var _ui: EstacionUI = null
var _camara: CamaraMapa = null
var _panel: PanelContainer
var _filtro_linea: OptionButton
var _filtro_solo_retraso: CheckBox
var _orden: OptionButton
var _contenedor_filas: VBoxContainer
var _titulo_total: Label

var _acumulador_refresco_seg: float = 0.0
var _pool_filas: Array = []   # Array[Dictionary]: ver _crear_fila_pool()


func configurar(trenes: Array, ui: EstacionUI, camara: CamaraMapa) -> void:
	_trenes = trenes
	_ui = ui
	_camara = camara


func _ready() -> void:
	layer = 10
	_panel = PanelContainer.new()
	# Panel a pantalla completa (con un margen): con decenas de trenes, cuanto
	# más sitio tenga la tabla, más cómodo es monitorizarla durante pruebas.
	_panel.set_anchors_preset(Control.PRESET_FULL_RECT)
	_panel.offset_left = MARGEN_PANTALLA
	_panel.offset_top = MARGEN_PANTALLA
	_panel.offset_right = -MARGEN_PANTALLA
	_panel.offset_bottom = -MARGEN_PANTALLA
	_panel.visible = false

	var sb := StyleBoxFlat.new()
	sb.bg_color = Color(0.08, 0.09, 0.11, 0.97)
	sb.set_corner_radius_all(10)
	sb.content_margin_left = 18.0
	sb.content_margin_right = 18.0
	sb.content_margin_top = 14.0
	sb.content_margin_bottom = 14.0
	_panel.add_theme_stylebox_override("panel", sb)
	add_child(_panel)

	var vb := VBoxContainer.new()
	vb.add_theme_constant_override("separation", 10)
	_panel.add_child(vb)

	var cab := HBoxContainer.new()
	vb.add_child(cab)
	var titulo := Label.new()
	titulo.text = "Trenes en marcha"
	titulo.add_theme_font_size_override("font_size", 22)
	cab.add_child(titulo)
	var sep := Control.new()
	sep.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	cab.add_child(sep)
	var btn_cerrar := Button.new()
	btn_cerrar.text = "✕  Cerrar"
	btn_cerrar.pressed.connect(cerrar)
	cab.add_child(btn_cerrar)

	# Barra de filtros, en su propio panel para separarla visualmente del
	# título y de la tabla que viene debajo.
	var sb_filtros := StyleBoxFlat.new()
	sb_filtros.bg_color = Color(1, 1, 1, 0.05)
	sb_filtros.set_corner_radius_all(6)
	sb_filtros.content_margin_left = 10.0
	sb_filtros.content_margin_right = 10.0
	sb_filtros.content_margin_top = 6.0
	sb_filtros.content_margin_bottom = 6.0
	var panel_filtros := PanelContainer.new()
	panel_filtros.add_theme_stylebox_override("panel", sb_filtros)
	vb.add_child(panel_filtros)

	var filtros := HBoxContainer.new()
	filtros.add_theme_constant_override("separation", 8)
	panel_filtros.add_child(filtros)

	var lbl_linea := Label.new()
	lbl_linea.text = "Línea:"
	lbl_linea.add_theme_font_size_override("font_size", 12)
	filtros.add_child(lbl_linea)
	_filtro_linea = OptionButton.new()
	_filtro_linea.add_item("Todas")
	for id_linea in Global.lineas.keys():
		_filtro_linea.add_item(str(id_linea))
	filtros.add_child(_filtro_linea)

	filtros.add_child(VSeparator.new())

	_filtro_solo_retraso = CheckBox.new()
	_filtro_solo_retraso.text = "Solo con retraso"
	filtros.add_child(_filtro_solo_retraso)

	filtros.add_child(VSeparator.new())

	var lbl_orden := Label.new()
	lbl_orden.text = "Ordenar por:"
	lbl_orden.add_theme_font_size_override("font_size", 12)
	filtros.add_child(lbl_orden)
	_orden = OptionButton.new()
	_orden.add_item("Línea")
	_orden.add_item("Retraso (mayor primero)")
	filtros.add_child(_orden)

	var relleno := Control.new()
	relleno.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	filtros.add_child(relleno)

	_titulo_total = Label.new()
	_titulo_total.add_theme_font_size_override("font_size", 12)
	_titulo_total.modulate = C_CABECERA
	filtros.add_child(_titulo_total)

	# Cabecera de la "tabla" (misma disposición de columnas que cada fila, ver
	# _fila_columnas(), para que quede todo alineado). El hueco final es solo
	# para cuadrar con la columna del botón "Ir" de cada fila (ver _crear_fila_pool).
	var cabecera := _fila_columnas("ID", "Línea", "Ubicación → Destino", "Retraso", true)
	var hueco_ir := Control.new()
	hueco_ir.custom_minimum_size = Vector2(ANCHO_COL_IR, 0)
	cabecera.add_child(hueco_ir)
	vb.add_child(cabecera)
	vb.add_child(HSeparator.new())

	# Con decenas de trenes en marcha, la lista suelta se saldría de la
	# pantalla: la metemos en un ScrollContainer con barra vertical que ocupa
	# todo el alto que le sobra al panel.
	var scroll := ScrollContainer.new()
	scroll.horizontal_scroll_mode = ScrollContainer.SCROLL_MODE_DISABLED
	scroll.size_flags_vertical = Control.SIZE_EXPAND_FILL
	_contenedor_filas = VBoxContainer.new()
	_contenedor_filas.add_theme_constant_override("separation", 0)
	_contenedor_filas.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	scroll.add_child(_contenedor_filas)
	vb.add_child(scroll)


## Construye una fila de 4 columnas (ID / línea / ubicación-destino / retraso)
## con los mismos anchos fijos siempre, tanto para la cabecera como para cada
## tren — así los textos quedan alineados en vertical como una tabla real.
func _fila_columnas(txt_id: String, txt_linea: String, txt_recorrido: String, txt_retraso: String, es_cabecera: bool) -> HBoxContainer:
	var fila := HBoxContainer.new()
	fila.add_theme_constant_override("separation", 14)

	var lbl_id := Label.new()
	lbl_id.text = txt_id
	lbl_id.custom_minimum_size = Vector2(ANCHO_COL_ID, 0)
	lbl_id.clip_text = true

	var lbl_linea := Label.new()
	lbl_linea.text = txt_linea
	lbl_linea.custom_minimum_size = Vector2(ANCHO_COL_LINEA, 0)
	lbl_linea.clip_text = true

	var lbl_recorrido := Label.new()
	lbl_recorrido.text = txt_recorrido
	lbl_recorrido.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	lbl_recorrido.clip_text = true

	var lbl_retraso := Label.new()
	lbl_retraso.text = txt_retraso
	lbl_retraso.custom_minimum_size = Vector2(ANCHO_COL_RETRASO, 0)
	lbl_retraso.horizontal_alignment = HORIZONTAL_ALIGNMENT_RIGHT

	var tam_fuente := 12 if es_cabecera else 14
	for lbl in [lbl_id, lbl_linea, lbl_recorrido, lbl_retraso]:
		lbl.add_theme_font_size_override("font_size", tam_fuente)
	if es_cabecera:
		for lbl in [lbl_id, lbl_linea, lbl_recorrido, lbl_retraso]:
			lbl.modulate = C_CABECERA
			lbl.add_theme_font_size_override("font_size", 12)
	else:
		lbl_id.add_theme_color_override("font_color", Color(0.6, 0.62, 0.68))
		lbl_linea.add_theme_color_override("font_color", Color(0.85, 0.87, 0.92))

	fila.add_child(lbl_id)
	fila.add_child(lbl_linea)
	fila.add_child(lbl_recorrido)
	fila.add_child(lbl_retraso)
	return fila


## Crea UNA fila del pool (envoltorio con cebra + sus 4 labels + el botón "Ir
## al tren") y la añade ya al contenedor. Solo se llama cuando el pool
## necesita crecer, nunca en cada refresco (ver cabecera del fichero).
##
## El botón se CREA una sola vez aquí, pero a qué tren apunta cambia en cada
## refresco (la fila i puede tocarle a un tren distinto según el filtro/orden
## del momento) — por eso _refrescar() desconecta la conexión anterior
## (guardada en "callable_ir") y conecta una nueva con `.bind(tren)` cada vez.
func _crear_fila_pool() -> Dictionary:
	var fila := _fila_columnas("", "", "", "", false)
	var btn_ir := Button.new()
	btn_ir.text = "📍 Ir"
	btn_ir.custom_minimum_size = Vector2(ANCHO_COL_IR, 0)
	fila.add_child(btn_ir)
	var envoltorio := PanelContainer.new()
	var sb := StyleBoxFlat.new()
	sb.content_margin_left = 4.0
	sb.content_margin_right = 4.0
	sb.content_margin_top = 3.0
	sb.content_margin_bottom = 3.0
	envoltorio.add_theme_stylebox_override("panel", sb)
	envoltorio.add_child(fila)
	_contenedor_filas.add_child(envoltorio)
	return {
		"env": envoltorio,
		"sb": sb,
		"lbl_id": fila.get_child(0) as Label,
		"lbl_linea": fila.get_child(1) as Label,
		"lbl_recorrido": fila.get_child(2) as Label,
		"lbl_retraso": fila.get_child(3) as Label,
		"btn_ir": btn_ir,
		"callable_ir": Callable(),
	}


func _obtener_fila_pool(i: int) -> Dictionary:
	if i < _pool_filas.size():
		return _pool_filas[i]
	var entrada := _crear_fila_pool()
	_pool_filas.append(entrada)
	return entrada


func abrir() -> void:
	_panel.visible = true
	# Refresco inmediato al abrir: si no, el usuario vería la tabla vacía o
	# desactualizada hasta que pase el primer INTERVALO_REFRESCO_SEG entero.
	_acumulador_refresco_seg = INTERVALO_REFRESCO_SEG
	_refrescar()


func cerrar() -> void:
	_panel.visible = false


func alternar() -> void:
	if _panel.visible:
		cerrar()
	else:
		abrir()


func _process(delta: float) -> void:
	if not _panel.visible:
		return
	_acumulador_refresco_seg += delta
	if _acumulador_refresco_seg < INTERVALO_REFRESCO_SEG:
		return
	_acumulador_refresco_seg = 0.0
	_refrescar()


func _refrescar() -> void:
	var linea_sel := "" if _filtro_linea.selected <= 0 else _filtro_linea.get_item_text(_filtro_linea.selected)
	var solo_retraso := _filtro_solo_retraso.button_pressed

	var filas: Array = []
	for t in _trenes:
		var tren := t as Tren
		if tren == null:
			continue
		var info := tren.info_debug()
		if linea_sel != "" and str(info["linea"]) != linea_sel:
			continue
		if solo_retraso and float(info["retraso_seg"]) <= RETRASO_ATENCION_SEG:
			continue
		filas.append({"tren": tren, "info": info})

	if _orden.selected == 1:
		filas.sort_custom(func(a: Dictionary, b: Dictionary) -> bool: return float((a["info"] as Dictionary)["retraso_seg"]) > float((b["info"] as Dictionary)["retraso_seg"]))
	else:
		filas.sort_custom(func(a: Dictionary, b: Dictionary) -> bool: return str((a["info"] as Dictionary)["linea"]) < str((b["info"] as Dictionary)["linea"]))

	# Estadísticas rápidas sobre TODA la flota (no solo lo filtrado), para
	# tener de un vistazo el estado general de la red sin tener que contar
	# filas a mano.
	var con_retraso := 0
	var criticos := 0
	var suma_retraso := 0.0
	for t in _trenes:
		var tren := t as Tren
		if tren == null:
			continue
		var r := float(tren.info_debug()["retraso_seg"])
		suma_retraso += r
		if r > RETRASO_CRITICO_SEG:
			criticos += 1
		elif r > RETRASO_ATENCION_SEG:
			con_retraso += 1
	var media_min := 0.0 if _trenes.is_empty() else (suma_retraso / _trenes.size()) / 60.0
	_titulo_total.text = "%d / %d trenes · %d con retraso · %d críticos · retraso medio %.1f min" % [
		filas.size(), _trenes.size(), con_retraso, criticos, media_min
	]

	for i in filas.size():
		var fila_datos: Dictionary = filas[i]
		var tren := fila_datos["tren"] as Tren
		var info: Dictionary = fila_datos["info"]
		var retraso_seg := float(info["retraso_seg"])
		var texto_retraso := "puntual" if retraso_seg <= RETRASO_ATENCION_SEG else "+%d min" % int(retraso_seg / 60.0)

		var entrada := _obtener_fila_pool(i)
		(entrada["lbl_id"] as Label).text = str(info["id_tren"])
		(entrada["lbl_linea"] as Label).text = str(info["linea"])
		(entrada["lbl_recorrido"] as Label).text = "%s → %s" % [info["ubicacion"], info["destino"]]
		var lbl_retraso := entrada["lbl_retraso"] as Label
		lbl_retraso.text = texto_retraso
		var color_retraso := C_PUNTUAL
		if retraso_seg > RETRASO_CRITICO_SEG:
			color_retraso = C_CRITICO
		elif retraso_seg > RETRASO_ATENCION_SEG:
			color_retraso = C_ATENCION
		lbl_retraso.add_theme_color_override("font_color", color_retraso)
		_rebindear_boton_ir(entrada, tren)

		# Franja de cebra: alterna un fondo apenas visible para que el ojo no
		# se pierda al leer filas con decenas de trenes.
		(entrada["sb"] as StyleBoxFlat).bg_color = C_FILA_PAR if i % 2 == 0 else C_FILA_IMPAR
		(entrada["env"] as Control).visible = true

	# Las filas del pool que sobran de un refresco con menos trenes visibles
	# que el anterior (por un filtro, p. ej.) se ocultan, NUNCA se destruyen:
	# así el pool no vuelve a crecer si luego hacen falta de nuevo.
	for i in range(filas.size(), _pool_filas.size()):
		(_pool_filas[i]["env"] as Control).visible = false


## El botón "Ir" de cada fila del pool es el MISMO Control entre refrescos,
## pero a qué tren apunta cambia cada vez (la fila i puede tocarle a un tren
## distinto tras reordenar/filtrar) — así que hay que desconectar la conexión
## de la vez anterior antes de conectar la nueva con `.bind(tren)`.
func _rebindear_boton_ir(entrada: Dictionary, tren: Tren) -> void:
	var boton := entrada["btn_ir"] as Button
	var anterior := entrada["callable_ir"] as Callable
	if anterior.is_valid() and boton.pressed.is_connected(anterior):
		boton.pressed.disconnect(anterior)
	var nueva := Callable(self, "_ir_al_tren").bind(tren)
	boton.pressed.connect(nueva)
	entrada["callable_ir"] = nueva


## "Ir al tren": desplaza la cámara hasta él, abre el panel de su estación si
## está parado dentro de una (en cualquier vía) y lanza el efecto de foco.
func _ir_al_tren(tren: Tren) -> void:
	if tren == null:
		return
	# Cerramos ESTE panel primero: si no, se queda tapando el mapa entero (es
	# a pantalla completa) y el usuario no vería ni la cámara desplazándose ni
	# el tren al que ha ido a parar.
	cerrar()
	if _camara != null:
		_camara.ir_a(tren.global_position)
	if _ui != null:
		var interlock := tren.interlock_actual()
		if interlock != null:
			_ui.abrir(interlock, _trenes)
	tren.add_child(PingLocalizador.new())
