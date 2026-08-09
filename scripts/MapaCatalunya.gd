extends Node2D
## ════════════════════════════════════════════════════════════════════════════
##  MapaCatalunya.gd  —  Mapa con vía doble continua y LOD por zoom
## ════════════════════════════════════════════════════════════════════════════
##  - De lejos (zoom < UMBRAL_VIA_DOBLE) se ve UNA sola línea central por línea.
##  - De cerca (zoom >= UMBRAL_VIA_DOBLE) se ven las DOS vías (rieles continuos,
##    pegaditos) y los semáforos triangulares sobre su vía.
##  - El cambio es un if sobre el zoom de la cámara, no un efecto de escala.
##
##  OFFSET_VIA debe COINCIDIR con el de Tren.gd para que el tren quede sobre su vía.
## ════════════════════════════════════════════════════════════════════════════

var capa_centros: Node2D     # línea única (vista de lejos)
var capa_rieles: Node2D      # las dos vías (vista de cerca)
var capa_via_unica: Node2D   # tramos de vía única: rail central + abanicos diagonales (vista de cerca)
var capa_semaforos: Node2D
var capa_semaforos_via_unica: Node2D   # semáforos de salida de vía única: SIEMPRE visibles, no sujetos al LOD
var capa_estaciones: Node2D
var capa_trenes: Node2D
var camara: CamaraMapa

var _registro: RegistroBloques
var _registro_via_unica: Dictionary = {}        # "id_a|id_b" (orden alfabético) -> TramoViaUnica, compartido entre los dos sentidos
var _curvas_por_linea: Dictionary = {}
var _rutas_ids: Dictionary = {}
var _cantones_por_tramo: Dictionary = {}        # "desde>hasta" -> Array[Bloque] (cadena completa, en orden desde->hasta)
var _cantones_por_linea: Dictionary = {}        # id_linea -> Array (por tramo: {"bloques_ida":Array[Bloque], "bloques_vuelta":Array[Bloque]})
var _centros: Array[Line2D] = []
var _rieles: Array[Line2D] = []

var _interlocks: Dictionary = {}                # id_estacion -> EstacionInterlock
var _lado_a_es_fin_por_linea: Dictionary = {}   # id_linea -> bool
var _trenes: Array = []                        # referencia viva (la usa el panel para ETA)
var _ui: EstacionUI
var _panel_trenes: TrenesPanel
var _horario: ScheduleManager
var _fin_dia: PanelFinDia

const OFFSET_VIA := 0.1             # separación de cada vía respecto al centro (px mundo)
const UMBRAL_VIA_DOBLE := 3.5       # zoom a partir del cual se ven las dos vías
const UMBRAL_SEMAFOROS := 14.0      # zoom a partir del cual se ven los semáforos (bastante más que las vías)
const ANCHO_LINEA_PX := 3.0         # grosor de la línea única, en px de pantalla
const ANCHO_VIA_PX := 2.0           # grosor de cada riel, en px de pantalla

## Cantonamiento proporcional: cuántos km de tramo "valen" un semáforo
## intermedio. Calibrado para que Barcelona-Fàbra i Puig <-> Barcelona-Torre
## Baró|Vallbona (~3.4 km, adyacentes en R7/R4) salga con EXACTAMENTE 2
## semáforos intermedios: 3.404277258418127 / 2.0.
const KM_POR_CANTON_INTERMEDIO := 1.702138629209064

## Agujas en diagonal de los tramos de vía única (ver _crear_tramo_via_unica_visual):
## distancia FIJA (en las mismas unidades de mundo que OFFSET_VIA, no una
## fracción del tramo) en la que las vías se unifican al entrar/salir de una
## estación con cruce. Tiene que ser pequeña de verdad: OFFSET_VIA = 0.1 es la
## separación entre rieles, así que un abanico de varias vías solo se abre
## unas pocas décimas a cada lado — si la distancia de convergencia fuera
## proporcional al tramo (como en el primer intento), en cualquier tramo corto
## o medio la "aguja" ocupaba el tramo ENTERO y se veía como un embudo continuo
## de punta a punta en vez de una unión rápida seguida de una recta.
const LARGO_AGUJA := 1.5


func _ready() -> void:
	RenderingServer.set_default_clear_color(Color(0.11, 0.12, 0.15))
	get_viewport().physics_object_picking = true

	# Global es un Autoload: sobrevive a un reload_current_scene() (p. ej.
	# "Repetir el día" en PanelFinDia), así que su reloj/KPI NO se resetean
	# solos al recargar esta escena. Como esta escena es siempre "empezar una
	# jornada nueva" (no hay partidas guardadas), lo forzamos aquí siempre.
	Global.reiniciar_jornada()
	Global.reiniciar_kpi()
	Global.establecer_velocidad("X1")   # por si el día anterior acabó en pausa (ver PanelFinDia)

	var tam := get_viewport_rect().size
	Global.preparar_proyeccion(tam, 80.0)

	_registro = RegistroBloques.new()
	_crear_interlocks()
	_ui = EstacionUI.new()
	add_child(_ui)
	# La cámara no depende de líneas/estaciones/horario (solo de la proyección,
	# ya lista), así que la creamos ya: TrenesPanel la necesita para "ir al tren".
	_crear_camara(tam)
	_panel_trenes = TrenesPanel.new()
	_panel_trenes.configurar(_trenes, _ui, camara)
	add_child(_panel_trenes)

	capa_centros = Node2D.new()
	capa_centros.name = "CapaCentros"
	add_child(capa_centros)
	capa_rieles = Node2D.new()
	capa_rieles.name = "CapaRieles"
	add_child(capa_rieles)
	capa_via_unica = Node2D.new()
	capa_via_unica.name = "CapaViaUnica"
	add_child(capa_via_unica)
	capa_semaforos = Node2D.new()
	capa_semaforos.name = "CapaSemaforos"
	add_child(capa_semaforos)
	capa_semaforos_via_unica = Node2D.new()
	capa_semaforos_via_unica.name = "CapaSemaforosViaUnica"
	add_child(capa_semaforos_via_unica)
	capa_estaciones = Node2D.new()
	capa_estaciones.name = "CapaEstaciones"
	add_child(capa_estaciones)
	capa_trenes = Node2D.new()
	capa_trenes.name = "CapaTrenes"
	add_child(capa_trenes)

	for id_linea in Global.lineas.keys():
		_crear_linea(str(id_linea))

	_crear_estaciones()
	_crear_horario()
	var hud := HUD.new()
	hud.configurar(_panel_trenes)
	add_child(hud)

	_fin_dia = PanelFinDia.new()
	_fin_dia.configurar(_trenes)
	add_child(_fin_dia)

	print("[Mapa] Catalunya construida: %d estaciones, %d líneas." % [
		Global.estaciones.size(), Global.lineas.size()
	])


func _process(_delta: float) -> void:
	if camara == null:
		return
	var z := camara.zoom.x

	# LOD: una línea de lejos, dos vías de cerca (decisión por umbral, no escala).
	# Los semáforos tienen su PROPIO umbral, más cercano todavía que el de las
	# vías: son muchos nodos (uno o varios por tramo y sentido, en las 8
	# líneas) y no hace falta verlos hasta que se está mirando de verdad un
	# cruce o una estación concreta.
	var doble := z >= UMBRAL_VIA_DOBLE
	var semaforos := z >= UMBRAL_SEMAFOROS
	capa_centros.visible = not doble
	capa_rieles.visible = doble
	capa_via_unica.visible = doble
	capa_semaforos.visible = semaforos
	capa_semaforos_via_unica.visible = semaforos

	# Grosores constantes en pantalla.
	var g_centro := ANCHO_LINEA_PX / z
	for c in _centros:
		c.width = g_centro
	var g_riel := ANCHO_VIA_PX / z
	for r in _rieles:
		r.width = g_riel


# ─────────────────────────────────────────────────────────────────────────────
#  LÍNEAS, VÍAS Y SEMÁFOROS
# ─────────────────────────────────────────────────────────────────────────────

func _crear_linea(id_linea: String) -> void:
	# El orden de estaciones ya NO es una heurística geográfica: viene de
	# horario_topologia.json, generado a partir de la secuencia REAL de
	# paradas de los horarios oficiales (ver generar_horario_oficial.py).
	# Es también el orden que usa ScheduleManager para los tramos/esperas, así
	# que la vía dibujada y el ritmo de los trenes SIEMPRE coinciden.
	var orden_ids := Global.get_orden_estaciones_linea(id_linea)
	var ruta: Array = []
	for sid in orden_ids:
		var est := Global.get_estacion(str(sid))
		if not est.is_empty():
			ruta.append(est)
	if ruta.size() < 2:
		return

	var terminales := Global.get_linea(id_linea).get("terminales", []) as Array
	var id_inicio := str(terminales[0]) if terminales.size() >= 2 else ""
	var id_fin_oficial := str(terminales[1]) if terminales.size() >= 2 else ""

	var ids: Array[String] = []
	for est in ruta:
		ids.append(str(est["id"]))
	_rutas_ids[id_linea] = ids

	# Una línea NO tiene "norte" ni "sur": tiene dos terminales. Para poder
	# identificarlas con una etiqueta corta y CONSISTENTE entre líneas que
	# comparten estación (para que sus vías principales se agrupen bien),
	# usamos la latitud como simple criterio de desempate interno arbitrario:
	# el extremo de mayor latitud se etiqueta "lado A", el otro "lado B". Esta
	# etiqueta es solo una clave interna — la interfaz nunca la muestra tal
	# cual, siempre enseña el nombre real de la estación terminal.
	# Los extremos para esta clasificación son las terminales OFICIALES (si
	# las hay), no simplemente el primer/último elemento de la ruta: alguna
	# línea (p. ej. R3) sigue teniendo, más allá de su terminal oficial, alguna
	# estación extra en el JSON (La Tor de Querol tras Puigcerdà) que de otro
	# modo acabaría siendo tratada por error como "la terminal".
	var e0 := id_inicio if id_inicio != "" else ids[0]
	var e1 := id_fin_oficial if id_fin_oficial != "" else ids[ids.size() - 1]
	var lat0 := float(Global.get_estacion(e0).get("lat", 0.0))
	var lat1 := float(Global.get_estacion(e1).get("lat", 0.0))
	# ¿Cuál de los dos terminales oficiales es el lado "A" (el de mayor latitud)?
	var e1_es_lado_a := lat1 >= lat0
	var id_lado_a := e1 if e1_es_lado_a else e0
	var id_lado_b := e0 if e1_es_lado_a else e1

	# ¿El extremo de ÍNDICE MÁS ALTO de `ids` (el orden REAL de la topología,
	# que viene de horario_topologia.json) es el lado A? OJO: este orden NO
	# tiene por qué coincidir con el de "terminales" en lineas.json — p. ej. en
	# R2 y R2_NORD la topología real empieza por el extremo norte (Granollers /
	# Maçanet) y acaba en el sur, justo al revés de como los lista su campo
	# "terminales". Asumir que terminales[0] cae en ids[0] (como se hacía antes)
	# etiquetaba el sentido norte como "B" en esas líneas mientras que R8, cuya
	# topología SÍ sigue el mismo orden que sus terminales, lo etiquetaba "A" —
	# con las tres líneas compartiendo estación en Montmeló/Granollers, esto
	# cruzaba las direcciones entre líneas y colapsaba la señalización. Por eso
	# comparamos las posiciones REALES de los dos terminales dentro de `ids`,
	# nunca el orden en que aparecen en "terminales".
	var idx_a := ids.find(id_lado_a)
	var idx_b := ids.find(id_lado_b)
	var lado_a_es_fin := idx_a >= idx_b if idx_a >= 0 and idx_b >= 0 else e1_es_lado_a
	_lado_a_es_fin_por_linea[id_linea] = lado_a_es_fin
	var term_a := str(Global.get_estacion(id_lado_a).get("nombre", ""))
	var term_b := str(Global.get_estacion(id_lado_b).get("nombre", ""))
	for sid in ids:
		var inter := _interlocks.get(sid, null) as EstacionInterlock
		if inter != null:
			inter.add_terminal("A", term_a)
			inter.add_terminal("B", term_b)

	# Curva central (la siguen los trenes) y sus vértices (las estaciones).
	var curva := Curve2D.new()
	for est in ruta:
		curva.add_point(Global.proyectar(float(est["lat"]), float(est["lon"])))
	_curvas_por_linea[id_linea] = curva

	var puntos := PackedVector2Array()
	for k in curva.point_count:
		puntos.append(curva.get_point_position(k))

	var color := Global.get_color_linea(id_linea)

	# Línea única (vista de lejos).
	var centro := Line2D.new()
	centro.points = puntos
	centro.width = ANCHO_LINEA_PX
	centro.default_color = color
	centro.joint_mode = Line2D.LINE_JOINT_ROUND
	centro.begin_cap_mode = Line2D.LINE_CAP_ROUND
	centro.end_cap_mode = Line2D.LINE_CAP_ROUND
	capa_centros.add_child(centro)
	_centros.append(centro)

	# ¿A partir de qué estación esta línea es de VÍA ÚNICA (ver TramoViaUnica.gd
	# y checklist de la R3)? "" o ausente en lineas.json = vía doble de punta a
	# punta, como todas las líneas hasta ahora (comportamiento sin cambios).
	var id_via_unica_desde := str(Global.get_linea(id_linea).get("via_unica_desde", ""))
	var idx_via_unica := ids.find(id_via_unica_desde) if id_via_unica_desde != "" else -1

	# Dos rieles continuos desplazados a cada lado (vista de cerca) para el
	# tramo de vía DOBLE (toda la línea si no es de vía única en ningún punto).
	var puntos_doble := puntos if idx_via_unica < 0 else puntos.slice(0, idx_via_unica + 1)
	if puntos_doble.size() >= 2:
		_crear_riel(_polilinea_desplazada(puntos_doble, 1.0), color)
		_crear_riel(_polilinea_desplazada(puntos_doble, -1.0), color)

	# Tramo de vía única: un solo raíl central con agujas en diagonal al
	# entrar/salir de cada estación con vías de cruce (ver
	# _crear_tramo_via_unica_visual — checklist #2, excepción de las estaciones
	# de alta montaña sin apartadero, tipo Toses, resuelta ahí mismo).
	if idx_via_unica >= 0:
		for i in range(idx_via_unica, ids.size() - 1):
			_crear_tramo_via_unica_visual(ids[i], ids[i + 1], puntos[i], puntos[i + 1], color, id_linea)

	# Semáforos de tramo (uno o varios por sentido, sobre su vía — ver
	# _crear_cantones_tramo: el tramo se subdivide en cantones proporcionales a
	# su distancia real). Cada cadena protege la entrada a su estación DESTINO
	# al final; qué lado (A/B) de esa estación representa se calcula con el
	# mismo criterio que Tren._dir_para(): moverse hacia índices crecientes de
	# la ruta es "hacia el lado A" si el final de la ruta ES el lado A, y
	# "hacia el lado B" en caso contrario (y viceversa para el sentido inverso).
	var dir_hacia_i1 := "A" if lado_a_es_fin else "B"   # ids[i] -> ids[i+1]
	var dir_hacia_i := "B" if lado_a_es_fin else "A"    # ids[i+1] -> ids[i]
	var cantones_linea: Array = []
	for i in (ids.size() - 1):
		var pa := puntos[i]
		var pb := puntos[i + 1]
		var es_via_unica := idx_via_unica >= 0 and i >= idx_via_unica
		var cadena_ida := _crear_cantones_tramo(ids[i], ids[i + 1], pa, pb, dir_hacia_i1, id_linea, es_via_unica)
		var cadena_vuelta := _crear_cantones_tramo(ids[i + 1], ids[i], pb, pa, dir_hacia_i, id_linea, es_via_unica)
		cantones_linea.append({"bloques_ida": cadena_ida, "bloques_vuelta": cadena_vuelta})
	_cantones_por_linea[id_linea] = cantones_linea


## Desplaza una polilínea una distancia OFFSET_VIA a un lado (signo +1 o -1),
## uniendo los vértices en inglete para que quede CONTINUA (sin cortes).
func _polilinea_desplazada(p: PackedVector2Array, signo: float) -> PackedVector2Array:
	var res := PackedVector2Array()
	var n := p.size()
	for i in n:
		if i == 0:
			var d := (p[1] - p[0]).normalized()
			res.append(p[0] + Vector2(-d.y, d.x) * (OFFSET_VIA * signo))
		elif i == n - 1:
			var d := (p[n - 1] - p[n - 2]).normalized()
			res.append(p[n - 1] + Vector2(-d.y, d.x) * (OFFSET_VIA * signo))
		else:
			var dp := (p[i] - p[i - 1]).normalized()
			var dn := (p[i + 1] - p[i]).normalized()
			var np := Vector2(-dp.y, dp.x)
			var nn := Vector2(-dn.y, dn.x)
			var miter := np + nn
			if miter.length() < 0.001:
				miter = nn
			miter = miter.normalized()
			# Compensamos el ángulo para mantener la separación constante.
			var coseno := miter.dot(nn)
			if coseno < 0.3:
				coseno = 0.3
			res.append(p[i] + miter * (OFFSET_VIA * signo / coseno))
	return res


func _crear_riel(puntos: PackedVector2Array, color: Color, capa: Node2D = null) -> void:
	var l := Line2D.new()
	l.points = puntos
	l.width = ANCHO_VIA_PX
	l.default_color = color
	l.joint_mode = Line2D.LINE_JOINT_ROUND
	l.begin_cap_mode = Line2D.LINE_CAP_ROUND
	l.end_cap_mode = Line2D.LINE_CAP_ROUND
	(capa if capa != null else capa_rieles).add_child(l)
	_rieles.append(l)


## Dibuja un tramo de VÍA ÚNICA (checklist #2): las vías se UNIFICAN rápido
## (en LARGO_AGUJA, una distancia fija y corta) justo al salir de una estación
## con vías de cruce (>= 2 vías EN SU CORREDOR de esta línea), y el RESTO del
## tramo —la inmensa mayoría, en cualquier hop de longitud normal— es un único
## raíl recto sin más adorno; al llegar a la siguiente estación se vuelve a
## abrir en abanico igual de rápido. Las estaciones estrictamente de vía única
## interior (Toses, Planoles, Urtx-Alp: 1 vía) NO abren abanico, la vía única
## pasa recta por su punto (excepción pedida explícitamente). Los Line2D
## resultantes se guardan en _rieles para que _process() les mantenga el
## mismo grosor constante en pantalla que al resto de vías.
func _crear_tramo_via_unica_visual(desde: String, hasta: String, p_desde: Vector2, p_hasta: Vector2, color: Color, id_linea: String) -> void:
	var d := (p_hasta - p_desde).normalized()
	var largo := (p_hasta - p_desde).length()
	var fan_len := minf(LARGO_AGUJA, largo * 0.35)   # los dos abanicos nunca se solapan en un tramo cortísimo

	var n_desde := _vias_de_cruce(desde, id_linea)
	var n_hasta := _vias_de_cruce(hasta, id_linea)

	var p_centro_desde := (p_desde + d * fan_len) if n_desde >= 2 else p_desde
	var p_centro_hasta := (p_hasta - d * fan_len) if n_hasta >= 2 else p_hasta

	if n_desde >= 2:
		_crear_abanico(p_desde, p_centro_desde, d, n_desde, color)
	if n_hasta >= 2:
		_crear_abanico(p_hasta, p_centro_hasta, d, n_hasta, color)

	_crear_riel(PackedVector2Array([p_centro_desde, p_centro_hasta]), color, capa_via_unica)


## Cuántas vías tiene, EN SU PROPIO CORREDOR (ver EstacionInterlock), la
## estación `sid` — 1 si no tiene enclavamiento o no declara corredores para
## esta línea (todas sus vías cuentan). Determina si esa estación abre
## abanico (>= 2) o la vía única pasa recta por su punto (1, como Toses).
func _vias_de_cruce(sid: String, id_linea: String) -> int:
	var inter := _interlocks.get(sid, null) as EstacionInterlock
	if inter == null:
		return 1
	return inter.vias_de_corredor(inter.corredor_de_linea(id_linea)).size()


## Un abanico de `num_vias` segmentos cortos que convergen, en diagonal,
## desde el offset de cada vía de la estación (`p_estacion`) hasta el punto
## único de la vía única (`p_centro`). `d` es la dirección "hacia fuera" del
## tramo (de la estación hacia el campo abierto); solo se usa para calcular
## la perpendicular de reparto, así que vale igual en el extremo `desde` que
## en el `hasta` (el conjunto de offsets es simétrico).
func _crear_abanico(p_estacion: Vector2, p_centro: Vector2, d: Vector2, num_vias: int, color: Color) -> void:
	var der := Vector2(-d.y, d.x)
	for i in num_vias:
		var offset := (float(i) - float(num_vias - 1) / 2.0) * OFFSET_VIA * 2.0
		var p0 := p_estacion + der * offset
		_crear_riel(PackedVector2Array([p0, p_centro]), color, capa_via_unica)


## Construye la CADENA de cantones (uno o varios Bloque seguidos) para el
## tramo desde->hasta, con un Semaforo por cada uno. El tramo se subdivide en
## tantos cantones intermedios como marque su distancia real (ver
## KM_POR_CANTON_INTERMEDIO) para que un tren no tenga que esperar a que TODO
## el tramo esté libre para arrancar, solo el primer trozo. Solo el ÚLTIMO
## cantón de la cadena (el que entra de verdad en la estación `hasta`) vigila
## la vía principal de esa estación (ver Bloque.configurar_destino); los
## intermedios son cantones "de vía abierta" sin estación — su semáforo
## depende solo del tránsito físico, porque Bloque sin interlock_destino
## configurado nunca considera ocupada ninguna vía principal (ver
## Bloque._via_principal_ocupada). Devuelve la cadena (Array[Bloque], en
## orden desde->hasta) para que Tren.gd sepa por qué cantones va pasando
## durante el tránsito (ver ScheduleManager/Tren._cantones).
##
## VÍA ÚNICA (`es_via_unica`): TODOS los bloques de la cadena (no solo el
## último) se asocian al mismo TramoViaUnica compartido con el sentido
## contrario (ver _obtener_token_via_unica) — así cualquier cantón detecta
## tráfico de cara, no solo el de entrada a la estación (checklist #4).
## Además, el PRIMER cantón (k==0, el que sale de la estación `desde`) se
## crea en modo "enclavamiento" (ver Semaforo.configurar_enclavamiento): va a
## la capa siempre-visible y su clic alterna el semáforo INTERNO de la propia
## estación de origen, en vez de un Bloque.modo manual independiente — mismo
## estado que el botón de FilaVia en el panel interior (checklist #3).
func _crear_cantones_tramo(desde: String, hasta: String, p_desde: Vector2, p_hasta: Vector2, direccion_llegada: String, id_linea: String, es_via_unica: bool = false) -> Array:
	var clave := desde + ">" + hasta
	if _cantones_por_tramo.has(clave):
		return _cantones_por_tramo[clave]

	var n := _calcular_num_cantones_intermedios(desde, hasta)
	var d := (p_hasta - p_desde).normalized()
	var der := Vector2(-d.y, d.x)
	var interlock_destino := _interlocks.get(hasta, null) as EstacionInterlock
	var corredor := interlock_destino.corredor_de_linea(id_linea) if interlock_destino != null else 0
	var interlock_origen := _interlocks.get(desde, null) as EstacionInterlock
	var token := _obtener_token_via_unica(desde, hasta) if es_via_unica else null

	var cadena: Array = []
	var anterior := p_desde
	for k in (n + 1):
		var es_ultimo := k == n
		var punto := p_hasta if es_ultimo else p_desde.lerp(p_hasta, float(k + 1) / float(n + 1))
		# El semáforo de un cantón va en su ENTRADA (el límite con el cantón
		# anterior), nunca en su punto medio: Bloque.en_rojo() refleja si ESTE
		# cantón (el que el semáforo protege, por delante de él en el sentido
		# de marcha) está ocupado, así que el semáforo debe encenderse justo
		# donde el tren entra a ocuparlo -- si se dibuja a medio cantón, se
		# pone en rojo bastante antes de que el tren llegue visualmente a él
		# (en cuanto entra por el otro extremo del mismo cantón), no "cuando
		# pasa por encima" como debe ser.
		var pos := anterior + der * OFFSET_VIA   # justo sobre el riel
		var bloque := _registro.bloque(clave, str(k))
		if es_ultimo:
			bloque.configurar_destino(interlock_destino, direccion_llegada, corredor)
		if token != null:
			bloque.configurar_via_unica(token, clave)
		var sem := Semaforo.new()
		if es_via_unica and k == 0 and interlock_origen != null:
			capa_semaforos_via_unica.add_child(sem)
			sem.configurar_enclavamiento(bloque, interlock_origen, direccion_llegada, id_linea, pos, d)
		else:
			capa_semaforos.add_child(sem)
			sem.configurar(bloque, pos, d)
		cadena.append(bloque)
		anterior = punto

	_cantones_por_tramo[clave] = cadena
	return cadena


## Testigo compartido de un tramo de vía única (ver TramoViaUnica.gd): UNA
## instancia para los dos sentidos, indexada por los dos ids SIN dirección
## (orden alfabético) para que "A>B" y "B>A" siempre encuentren el mismo objeto.
func _obtener_token_via_unica(desde: String, hasta: String) -> TramoViaUnica:
	var clave := (desde + "|" + hasta) if desde < hasta else (hasta + "|" + desde)
	if not _registro_via_unica.has(clave):
		_registro_via_unica[clave] = TramoViaUnica.new()
	return _registro_via_unica[clave]


## Cuántos semáforos intermedios le tocan a un tramo, proporcional a su
## distancia geográfica real (ver KM_POR_CANTON_INTERMEDIO).
func _calcular_num_cantones_intermedios(desde: String, hasta: String) -> int:
	var e_desde := Global.get_estacion(desde)
	var e_hasta := Global.get_estacion(hasta)
	if e_desde.is_empty() or e_hasta.is_empty():
		return 0
	var km := Global.distancia_km(float(e_desde["lat"]), float(e_desde["lon"]), float(e_hasta["lat"]), float(e_hasta["lon"]))
	return maxi(0, int(round(km / KM_POR_CANTON_INTERMEDIO)))


# ─────────────────────────────────────────────────────────────────────────────
#  ESTACIONES Y TRENES
# ─────────────────────────────────────────────────────────────────────────────

func _crear_interlocks() -> void:
	for datos in Global.estaciones:
		var sid := str(datos["id"])
		var nom := str(datos.get("nombre", sid))
		var nv := int(datos.get("vias", 1))
		var corredores := datos.get("corredores", []) as Array
		_interlocks[sid] = EstacionInterlock.new(sid, nom, nv, corredores)


func _crear_estaciones() -> void:
	# Estaciones que son terminal de ALGUNA línea (su punto se dibuja más grande).
	var terminales_red: Dictionary = {}
	for datos_linea in Global.lineas.values():
		var terminales := (datos_linea as Dictionary).get("terminales", []) as Array
		for t in terminales:
			terminales_red[str(t)] = true

	for datos in Global.estaciones:
		var est := Estacion.new()
		est.position = Global.proyectar(float(datos["lat"]), float(datos["lon"]))
		capa_estaciones.add_child(est)
		var sid := str(datos["id"])
		var es_terminal := bool(terminales_red.get(sid, false))
		est.configurar(datos, _interlocks.get(sid, null) as EstacionInterlock, _ui, _trenes, es_terminal)


func _crear_horario() -> void:
	_horario = ScheduleManager.new()
	add_child(_horario)
	_horario.configurar(capa_trenes, _curvas_por_linea, _rutas_ids, _interlocks,
		_lado_a_es_fin_por_linea, _trenes, _cantones_por_linea)


func _crear_camara(tam: Vector2) -> void:
	camara = CamaraMapa.new()
	camara.name = "Camara"
	var foco := Global.proyectar_estacion("SANTS")
	if foco == Vector2.ZERO:
		foco = tam / 2.0
	camara.position = foco
	camara.zoom = Vector2(2.5, 2.5)
	add_child(camara)
	camara.make_current()
