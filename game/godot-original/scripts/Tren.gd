class_name Tren
extends Area2D
## ════════════════════════════════════════════════════════════════════════════
##  Tren.gd  —  Tren con vía doble: circula SIEMPRE por la derecha de su sentido
## ════════════════════════════════════════════════════════════════════════════
##  Ya no usamos PathFollow2D: nos posicionamos a mano sobre la curva central de
##  la línea y nos desplazamos perpendicularmente "a la derecha" del sentido de
##  marcha. Así circulamos por la vía correcta en cada sentido (vía doble).
##
##  RITMO: el tren NUNCA consulta el reloj del juego mientras circula. Cada
##  tramo tiene una velocidad constante = distancia del tramo / tiempo de tramo
##  (horario_topologia.json) — el horario es una GUÍA de ritmo, no una ley: si
##  un semáforo o una vía ocupada lo retiene, simplemente sale más tarde.
##
##  TRENES FINITOS Y PERSISTENTES: cada Tren es un objeto que vive TODA la
##  jornada, no uno por servicio. Al llegar a la estación de destino de su
##  servicio asignado, se PARA a esperar la próxima asignación (no desaparece):
##  se aparta a una vía NO principal (así no bloquea la llegada de otros
##  trenes) y avisa a ScheduleManager de que está libre. Si el jugador retiene
##  este tren en algún punto de la ruta, sencillamente nunca llega a la
##  terminal, nunca queda "disponible" y ScheduleManager no tendrá con qué
##  cubrir la siguiente salida en sentido contrario — el retraso es real y no
##  se inventa un tren fantasma para disimularlo.
##
##  RETRASO: al asignarle un nuevo servicio, compara la hora actual con la hora
##  a la que debería haber salido y se queda con esa diferencia (_retraso_seg),
##  que se dibuja como una pequeña etiqueta sobre el tren. Solo se calcula en
##  terminales (no en cada parada intermedia): es barato y es donde de verdad
##  se nota si el sistema va acumulando tarde.
##
##  REPOSICIONAMIENTO: cuando una terminal no tiene vías para aparcar TODOS
##  los trenes que necesitará a lo largo del día, ScheduleManager aparca el
##  sobrante en una estación cercana con vías libres y, con margen de sobra
##  antes de su primera salida real, lo manda "en vacío" (sin parar en
##  intermedias, ver asignar_reposicionamiento()) hasta la terminal saturada.
##
##  IMPORTANTE: OFFSET_VIA debe COINCIDIR con el de MapaCatalunya.gd (donde se
##  dibujan los rieles), para que el tren quede justo encima de su vía.
## ════════════════════════════════════════════════════════════════════════════

const LARGO := 18.0
const ANCHO := 8.0
const OFFSET_VIA := 0.1            # separación de cada vía respecto al centro (px mundo)
const GIRO_MINIMO_SEG := 5.0 * 60.0 # tiempo mínimo para dar la vuelta en una terminal, aunque llegue tarde
const UMBRAL_CEDER_PASO_SEG := 5.0 * 60.0 # tiempo bloqueado (más allá de la parada normal) antes de apartarse

enum Estado { MARCHA, PARADO }

## Se emite cuando el tren llega a su terminal de destino, termina el giro
## mínimo y queda aparcado esperando la próxima asignación. ScheduleManager
## la escucha para saber qué trenes tiene libres en cada estación.
signal disponible_en_terminal(tren: Tren, id_estacion: String)

## Se emite cuando llevamos ya UMBRAL_CEDER_PASO_SEG aparcados en la vía
## PRINCIPAL sin servicio asignado y sin apartadero propio libre (ver
## _esperar_asignacion): seguir así bloquea la entrada de toda la línea a esta
## estación, así que pedimos ayuda para reubicarnos en la estación libre más
## cercana, igual que la dispersión nocturna pero en cualquier momento del día.
signal necesita_reubicacion(tren: Tren, id_estacion: String)

var id_tren: String = ""
var id_linea: String = ""
var modelo_id: String = ""
var color: Color = Color.WHITE

var _curva: Curve2D = null
var _ruta_ids: Array[String] = []
var _paradas: Array[float] = []
var _tramos: Array = []          # tramos[i] = segundos entre _ruta_ids[i] y _ruta_ids[i+1] (constante, de horario_topologia.json)
var _esperas: Array = []         # esperas[i] = segundos de parada comercial en _ruta_ids[i]
var _progreso: float = 0.0
var _velocidad_actual_px_seg: float = 0.0
var _idx_objetivo: int = 0
var _idx_origen_servicio: int = 0
var _idx_destino_final: int = 0     # == _idx_objetivo cuando el tren está aparcado SIN asignación
var _direccion: float = 1.0
var _estado := Estado.MARCHA
var _tiempo_parada: float = 0.0
var _bloque_actual: Bloque = null
var _notificado_disponible: bool = false
var _retraso_seg: float = 0.0
var _en_reposicionamiento: bool = false   # viaje "en vacío" hacia una terminal saturada: no para en intermedias
var _tiempo_bloqueado_seg: float = 0.0    # cuánto llevamos intentando partir sin conseguirlo (ver _ceder_paso_si_procede)

# --- Cantonamiento intermedio (ver MapaCatalunya._crear_cantones_tramo) ---
# Un tramo entre dos estaciones puede estar dividido en varios cantones
# seguidos (proporcional a su distancia real), no uno solo: así un tren no
# tiene que esperar a que TODO el tramo esté libre, solo el trozo que tiene
# delante. _cantones[i] = tramo entre _ruta_ids[i] y _ruta_ids[i+1] -> ambas
# direcciones ya resueltas (ver Tren.configurar).
var _cantones: Array = []
var _cantones_tramo: Array = []   # cadena (Array[Bloque]) del tramo que estamos cruzando ahora mismo, en nuestro sentido
var _canton_offsets: Array = []   # offset de progreso donde ACABA cada cantón de _cantones_tramo (mismo tamaño)
var _canton_actual: int = 0       # índice en _cantones_tramo/_canton_offsets del cantón que ocupamos ahora mismo

# --- Enclavamiento de estaciones ---
var _interlocks: Dictionary = {}                 # id_estacion -> EstacionInterlock
var _lado_a_es_fin: bool = true                  # ¿el extremo FINAL de la ruta (índice más alto) es el lado "A"?
var _estacion_actual: EstacionInterlock = null   # estación donde estamos parados
var _via_actual: int = -1                        # vía que ocupamos al estar parados
var _dir_salida: String = "A"                    # lado de salida ("A"/"B", el extremo de línea al que nos dirigimos)
var _esperando_via: bool = false                 # llegamos pero la vía estaba ocupada

# --- Pasajeros a bordo (modelo por lotes, SIN nodo/objeto por pasajero) ---
# id_estacion_destino -> cantidad de pasajeros que lleva este tren hacia ahí.
# Se rellena al absorber lotes de una estación (ver ScheduleManager/Estacion,
# checklist de pasajeros) y se vacía al llegar cada uno a su propio destino.
var pasajeros_por_destino: Dictionary = {}

func pasajeros_a_bordo() -> int:
	var total := 0
	for cantidad in pasajeros_por_destino.values():
		total += int(cantidad)
	return total

func capacidad_maxima() -> int:
	return int(Global.get_modelo_tren(modelo_id).get("capacidad", 0))

# --- Hover: destino + retraso en un tooltip flotante (ver TrenTooltip.gd) ---
var _hover: bool = false
var _tooltip: TrenTooltip = null


func configurar(linea: String, id_modelo: String, curva: Curve2D, ruta_ids: Array, tramos: Array, esperas: Array,
		interlocks: Dictionary, lado_a_es_fin: bool, cantones: Array) -> void:
	id_linea = linea
	modelo_id = id_modelo
	color = Global.get_color_linea(linea)
	_curva = curva
	_tramos = tramos
	_esperas = esperas
	_interlocks = interlocks
	_lado_a_es_fin = lado_a_es_fin
	_cantones = cantones
	_ruta_ids.assign(ruta_ids)
	_calcular_paradas()
	_configurar_hover()
	queue_redraw()


## Área de colisión (para detectar el ratón encima) + el tooltip flotante que
## se muestra mientras dura el hover.
func _configurar_hover() -> void:
	var col := CollisionShape2D.new()
	var forma := RectangleShape2D.new()
	forma.size = Vector2(LARGO, ANCHO)
	col.shape = forma
	add_child(col)
	input_pickable = true
	# Solo nos interesa la detección del ratón (picking), no la superposición
	# físicas entre Areas: con decenas de trenes en marcha, dejar monitoring
	# activo obligaría al motor a comprobar solapes contra cada estación y
	# semáforo (también Area2D) en cada fotograma sin que nadie escuche esa señal.
	monitoring = false
	monitorable = false
	mouse_entered.connect(_on_mouse_entered)
	mouse_exited.connect(_on_mouse_exited)

	_tooltip = TrenTooltip.new()
	add_child(_tooltip)


func _on_mouse_entered() -> void:
	_hover = true


func _on_mouse_exited() -> void:
	_hover = false


## Un tren aparcado en un apartadero (vía NO principal) es tráfico "invisible"
## de fondo, no algo relevante en el mapa geográfico: lo ocultamos (y dejamos
## de recibir el ratón) mientras dure esa espera. Circulando, esperando vía
## principal libre o parado en la propia vía principal, se sigue viendo igual
## que siempre.
func _actualizar_visibilidad() -> void:
	var oculto := _estado == Estado.PARADO and _estacion_actual != null and _via_actual >= 0 \
		and not _estacion_actual.es_principal(_via_actual, "A") and not _estacion_actual.es_principal(_via_actual, "B")
	visible = not oculto
	input_pickable = not oculto
	if oculto:
		_hover = false


## Mantiene el tooltip sincronizado (posición/escala constante en pantalla,
## sin heredar la rotación del tren) y con el texto de destino + retraso al día.
func _actualizar_tooltip() -> void:
	if _tooltip == null:
		return
	_tooltip.visible = _hover
	if not _hover:
		return

	var cam := get_viewport().get_camera_2d()
	var esc := Vector2.ONE / cam.zoom if cam != null else Vector2.ONE
	_tooltip.global_position = global_position + Vector2(0.0, -ANCHO) * esc.y
	_tooltip.scale = esc

	var texto_retraso := "puntual" if _retraso_seg <= 60.0 else "+%d min" % int(_retraso_seg / 60.0)
	_tooltip.fijar_texto("%s · Destino: %s\nRetraso: %s\nOcupación: %d / %d pas" % [
		id_linea, _nombre_estacion(_idx_destino_final), texto_retraso, pasajeros_a_bordo(), capacidad_maxima()
	])


## Nombre real de la estación en el índice `idx` de la ruta ("?" si no existe).
func _nombre_estacion(idx: int) -> String:
	if idx < 0 or idx >= _ruta_ids.size():
		return "?"
	return str(Global.get_estacion(_ruta_ids[idx]).get("nombre", _ruta_ids[idx]))


## Resumen para depuración (panel de trenes / volcado por consola, ver
## TrenesPanel.gd y ScheduleManager._volcar_estado_trenes()): línea, dónde está
## ahora mismo (parado en una estación o circulando entre dos), destino final
## de su servicio actual y retraso.
func info_debug() -> Dictionary:
	var ubicacion := ""
	if _estado == Estado.PARADO:
		ubicacion = "parado en " + _nombre_estacion(_idx_objetivo)
	else:
		var anterior := _idx_objetivo - int(_direccion)
		ubicacion = "entre %s y %s" % [_nombre_estacion(anterior), _nombre_estacion(_idx_objetivo)]
	return {
		"id_tren": id_tren,
		"linea": id_linea,
		"ubicacion": ubicacion,
		"destino": _nombre_estacion(_idx_destino_final),
		"retraso_seg": _retraso_seg,
	}


## Crea el tren YA aparcado en la estación `idx_estacion` (índice en
## _ruta_ids), sin ningún servicio asignado todavía: se anuncia disponible de
## inmediato para que ScheduleManager le dé la primera salida que le toque.
func nacer_en_estacion(idx_estacion: int) -> void:
	if idx_estacion < 0 or idx_estacion >= _paradas.size():
		return
	_idx_objetivo = idx_estacion
	_idx_destino_final = idx_estacion
	_idx_origen_servicio = idx_estacion
	_progreso = _paradas[idx_estacion]
	_estado = Estado.PARADO
	_tiempo_parada = 0.0
	var sid := _ruta_ids[idx_estacion]
	_estacion_actual = _interlocks.get(sid, null) as EstacionInterlock
	_via_actual = -1
	_esperando_via = false
	_notificado_disponible = false
	_ocupar_via_de_partida_inicial()
	_actualizar_transform()
	queue_redraw()


## Al nacer no llegamos por ningún cantón que haya que "cerrar" ocupando la
## vía principal (eso es solo para trenes que de verdad acaban de circular
## hasta aquí): un tren que solo espera su horario NUNCA debe ocupar una vía
## principal (bloquearía el cantón de ese sentido para cualquier otro tren,
## como pasaba en Montmeló). Por eso solo usamos un apartadero real.
## ScheduleManager ya descarta, al repartir/reposicionar la flota, cualquier
## estación sin apartaderos libres (ver _capacidad_directa()), así que este
## caso no debería darse nunca en la práctica; si aun así ocurre, avisamos por
## consola en vez de colapsar la señalización en silencio.
func _ocupar_via_de_partida_inicial() -> void:
	if _estacion_actual == null:
		return
	var via := _estacion_actual.via_libre_no_principal(id_linea)
	if via < 0:
		push_warning("[Tren] Sin apartadero libre en %s para un tren de %s: se aparca en vía principal (revisar capacidad)." % [_estacion_actual.id, id_linea])
		via = _estacion_actual.primera_via_libre(id_linea)
	if via >= 0:
		_estacion_actual.ocupar(via, self)
		_via_actual = via


## ScheduleManager nos asigna la próxima salida: viajar hasta `idx_destino`,
## que debería haber salido a `hora_salida_prevista` (segundos de juego). El
## tren YA está físicamente en su estación de origen (llegó y quedó aparcado
## ahí antes), así que solo hace falta fijar hacia dónde vamos y arrancar.
func asignar_servicio(idx_destino: int, hora_salida_prevista: float) -> void:
	_idx_origen_servicio = _idx_objetivo
	_idx_destino_final = idx_destino
	_direccion = 1.0 if idx_destino > _idx_objetivo else -1.0
	_dir_salida = _dir_para(_direccion)
	# Algunos servicios del GTFS original salen antes de las 05:00 (el arranque
	# de la jornada simulada): comparar directamente contra esa hora_salida_prevista
	# los marcaría como "tarde" desde el primer fotograma, aunque en la práctica
	# ese servicio no podía salir antes de que empezara nuestra jornada. Usamos
	# como referencia la hora prevista o el inicio de jornada, lo que sea más tarde.
	var referencia := maxf(hora_salida_prevista, Global.HORA_INICIO_SEG)
	_retraso_seg = maxf(0.0, Global.tiempo_juego_seg - referencia)
	_notificado_disponible = false
	_tiempo_parada = 0.0
	# Arrancamos un servicio nuevo desde donde ya estábamos aparcados: recoge
	# aquí también, igual que en cada parada intermedia (_llegar_a_estacion) —
	# si no, los pasajeros que esperaban en la propia terminal de origen nunca
	# subirían al primer tren que sale de verdad hacia ellos. Se hace ANTES de
	# registrar el KPI para que el impacto de este retraso se calcule con la
	# gente que de verdad viaja en este tren, no con un hueco vacío.
	_recoger_pasajeros()
	# KPI de la red (minutos de retraso × pasajeros afectados, ver Global.gd):
	# cada vez que un tren arranca un servicio real con retraso, contamos a
	# los pasajeros que lleva REALMENTE a bordo en ese instante (no la
	# capacidad máxima del modelo, que sobreestimaba el impacto de un tren
	# medio vacío igual que el de uno lleno).
	if _retraso_seg > 0.0:
		Global.registrar_retraso(_retraso_seg / 60.0, pasajeros_a_bordo())
	queue_redraw()


## Viaje "en vacío" (sin servicio comercial) hasta `idx_destino`, sin parar en
## las estaciones intermedias: lo usa ScheduleManager para reposicionar, con
## tiempo de sobra antes de su primera salida real, un tren aparcado en una
## estación con vías libres hacia la terminal donde de verdad hace falta pero
## no caben todos aparcados a la vez (p. ej. Terrassa Nord reforzado desde
## Terrassa Est). No cuenta como retraso: no es un servicio comercial.
func asignar_reposicionamiento(idx_destino: int) -> void:
	_idx_origen_servicio = _idx_objetivo
	_idx_destino_final = idx_destino
	_direccion = 1.0 if idx_destino > _idx_objetivo else -1.0
	_dir_salida = _dir_para(_direccion)
	_en_reposicionamiento = true
	_notificado_disponible = false
	_tiempo_parada = 0.0
	queue_redraw()


func _calcular_paradas() -> void:
	if _curva == null:
		return
	_paradas.clear()
	# El índice k corresponde a _ruta_ids[k] (no ordenamos).
	for k in _curva.point_count:
		_paradas.append(_curva.get_closest_offset(_curva.get_point_position(k)))


func _process(delta: float) -> void:
	_actualizar_transform()
	_actualizar_visibilidad()
	_actualizar_tooltip()

	if Global.multiplicador_velocidad <= 0.0:
		return
	if _paradas.size() < 2:
		return

	var seg_juego := delta * Global.FACTOR_BASE_TIEMPO * Global.multiplicador_velocidad

	if _estado == Estado.PARADO:
		# Si llegamos pero la vía principal estaba ocupada, esperamos a que se libere.
		if _esperando_via:
			_intentar_ocupar_via()
			if _esperando_via:
				return
		if _tiempo_parada > 0.0:
			_tiempo_parada -= seg_juego
			return
		_tiempo_bloqueado_seg += seg_juego
		if _idx_objetivo == _idx_destino_final:
			_esperar_asignacion()
			return
		_intentar_partir()
		return

	_progreso += _direccion * _velocidad_frenada() * seg_juego
	_actualizar_canton_intermedio()

	var destino := _paradas[_idx_objetivo]
	if _direccion > 0.0 and _progreso >= destino:
		_progreso = destino
		_llegar_a_estacion()
	elif _direccion < 0.0 and _progreso <= destino:
		_progreso = destino
		_llegar_a_estacion()


## Calcula posición (sobre la vía de la derecha), rotación y escala constante.
func _actualizar_transform() -> void:
	if _curva == null:
		return
	var largo := _curva.get_baked_length()
	if largo <= 0.0:
		return
	var prog := clampf(_progreso, 0.0, largo)
	var centro := _curva.sample_baked(prog)
	var a := _curva.sample_baked(clampf(prog - 1.0, 0.0, largo))
	var b := _curva.sample_baked(clampf(prog + 1.0, 0.0, largo))
	var tang := b - a
	if tang.length() < 0.001:
		tang = Vector2.RIGHT
	tang = tang.normalized()

	# "Derecha" del sentido de marcha (en pantalla, con la Y hacia abajo).
	var derecha := Vector2(-tang.y, tang.x)
	position = centro + derecha * (OFFSET_VIA * _direccion)
	rotation = (tang * _direccion).angle()

	var cam := get_viewport().get_camera_2d()
	if cam != null:
		scale = Vector2.ONE / cam.zoom


func _llegar_a_estacion() -> void:
	_estado = Estado.PARADO
	_tiempo_bloqueado_seg = 0.0
	var es_terminal_final := _idx_objetivo == _idx_destino_final
	if es_terminal_final:
		_tiempo_parada = GIRO_MINIMO_SEG
		_en_reposicionamiento = false   # llegamos a destino: a partir de aqui es un aparcamiento normal
	elif _en_reposicionamiento:
		_tiempo_parada = 0.0   # en vacio: no paramos comercialmente en las intermedias
	else:
		_tiempo_parada = float(_esperas[_idx_objetivo]) if _idx_objetivo < _esperas.size() else 20.0
	_dir_salida = _dir_para(_direccion)
	var sid := ""
	if _idx_objetivo >= 0 and _idx_objetivo < _ruta_ids.size():
		sid = _ruta_ids[_idx_objetivo]
	_estacion_actual = _interlocks.get(sid, null) as EstacionInterlock
	_via_actual = -1
	_esperando_via = false
	# Liberamos ya el cantón exterior por el que llegamos: en cuanto el tren
	# alcanza el andén deja de ocupar la vía ABIERTA entre estaciones, sea cual
	# sea la vía de la estación en la que acabe parado (principal o apartadero).
	# A partir de aquí, que ese cantón deba verse en rojo o no depende SOLO de
	# si la vía PRINCIPAL de este lado está ocupada (ver Bloque._via_principal_ocupada),
	# no de que este tren siga aquí parado.
	if _bloque_actual != null:
		_bloque_actual.ocupado = false
		_bloque_actual.liberar_via_unica()
	_cantones_tramo = []
	_canton_offsets = []
	_canton_actual = 0

	# Intercambio de pasajeros (ver EstacionInterlock/checklist de pasajeros):
	# descargamos SIEMPRE a quien llegue a su destino aquí; si seguimos de
	# largo, además recogemos lo que la estación tenga esperando en nuestro
	# sentido, hasta llenar el hueco libre que nos quede.
	if sid != "":
		pasajeros_por_destino.erase(sid)
	if not es_terminal_final:
		_recoger_pasajeros()

	if es_terminal_final:
		# Fin de servicio (final de línea de verdad o giro corto intermedio):
		# vamos DIRECTOS a un apartadero, nunca a la vía principal, ni un
		# instante — no nos importa qué vía en concreto porque no seguimos de
		# largo (ver _ocupar_via_secundaria_obligatoria y, en el otro extremo
		# del cantón, Bloque._destino_es_terminal, que ya no exige la
		# principal libre para dejarnos entrar, solo ALGUNA vía libre).
		_ocupar_via_secundaria_obligatoria()
	else:
		# Parada intermedia normal: seguimos de largo, así que sí necesitamos
		# LA vía principal de nuestro lado (así el cantón por el que veníamos
		# se pone en rojo para el que venga detrás, como es debido).
		_intentar_ocupar_via()
	queue_redraw()


## Absorbe, de la estación en la que estamos parados, los lotes de pasajeros
## cuyo destino está en NUESTRO sentido de circulación (delante de nosotros,
## sin pasarnos del final de este servicio) y quepan en el hueco libre que
## nos quede -- instantáneo, sin nodo/objeto por pasajero (ver checklist).
func _recoger_pasajeros() -> void:
	if _estacion_actual == null:
		return
	var espacio := capacidad_maxima() - pasajeros_a_bordo()
	if espacio <= 0:
		return
	for id_destino in (_estacion_actual.pasajeros_por_destino.keys() as Array).duplicate():
		if espacio <= 0:
			break
		if not _destino_alcanzable(str(id_destino)):
			continue
		var recogidos := _estacion_actual.extraer_pasajeros(str(id_destino), espacio)
		if recogidos > 0:
			pasajeros_por_destino[id_destino] = int(pasajeros_por_destino.get(id_destino, 0)) + recogidos
			espacio -= recogidos


## ¿Un pasajero con destino `id_destino` puede viajar en este tren desde AQUÍ
## MISMO, en el sentido en el que vamos a partir, sin pasarse del final de
## nuestro servicio actual? Solo cuenta si esa estación está en nuestra
## propia línea (un tren no hace trasbordos que el simulador no modela).
func _destino_alcanzable(id_destino: String) -> bool:
	var idx_destino := _ruta_ids.find(id_destino)
	if idx_destino < 0:
		return false
	if _direccion > 0.0:
		return idx_destino > _idx_objetivo and idx_destino <= _idx_destino_final
	return idx_destino < _idx_objetivo and idx_destino >= _idx_destino_final


## Intenta ocupar la vía principal del sentido de salida. Si la ocupa otro tren,
## quedamos a la espera (esto mantiene "en rojo" el tramo por el que llegamos).
func _intentar_ocupar_via() -> void:
	if _estacion_actual == null:
		return
	var via := _estacion_actual.via_principal(_dir_salida, id_linea)
	var ocup: Variant = _estacion_actual.tren_en(via)
	if ocup == null or ocup == self:
		_estacion_actual.ocupar(via, self)
		_via_actual = via
		_esperando_via = false
	else:
		_esperando_via = true


## Para un tren que TERMINA SU SERVICIO aquí (final de línea de verdad o giro
## corto intermedio, ver _llegar_a_estacion): se desvía obligatoriamente a un
## apartadero, nunca a la vía principal, ni un instante — no le hace falta
## para nada, porque no va a seguir de largo. Si excepcionalmente no hay
## ningún apartadero libre (no debería pasar: el reparto de flota de
## ScheduleManager ya reserva capacidad de apartadero para esto, ver
## _capacidad_directa()), avisamos por consola en vez de colapsar la
## señalización en silencio — igual que ya hace _ocupar_via_de_partida_inicial().
func _ocupar_via_secundaria_obligatoria() -> void:
	if _estacion_actual == null:
		return
	var via := _estacion_actual.via_libre_no_principal(id_linea)
	if via < 0:
		push_warning("[Tren] Fin de servicio en %s sin apartadero libre para %s: se aparca en vía principal (revisar capacidad)." % [_estacion_actual.id, id_linea])
		via = _estacion_actual.primera_via_libre(id_linea)
	if via >= 0:
		_estacion_actual.ocupar(via, self)
		_via_actual = via
	_esperando_via = false


## Ya cumplimos el giro mínimo en nuestra terminal y no tenemos servicio
## asignado: nos apartamos a una vía NO principal (si hay alguna libre) para
## no seguir bloqueando la llegada de otros trenes, y avisamos de que estamos
## disponibles. ScheduleManager nos llamará a asignar_servicio() cuando toque.
## (Ya llegamos directos a un apartadero en _llegar_a_estacion, ver
## _ocupar_via_secundaria_obligatoria — pero no estorba dejarlo genérico aquí
## también: si ya estamos en apartadero, la condición de abajo simplemente no entra).
func _esperar_asignacion() -> void:
	if _estacion_actual == null:
		return
	if _mover_a_apartadero_si_hay():
		_tiempo_bloqueado_seg = 0.0
	if not _notificado_disponible:
		_notificado_disponible = true
		disponible_en_terminal.emit(self, _estacion_actual.id)
		return
	# Ya estamos registrados como disponibles (ScheduleManager ya nos tiene en
	# su cola de esta estación). Si seguimos aparcados en la vía PRINCIPAL
	# (no había apartadero libre ni al llegar ni en ningún intento posterior)
	# y llevamos ya UMBRAL_CEDER_PASO_SEG así, pedimos ayuda para reubicarnos
	# en vez de seguir bloqueando indefinidamente la entrada de toda la línea
	# a esta estación (ver necesita_reubicacion / ScheduleManager._on_necesita_reubicacion).
	if esta_en_via_principal() and _tiempo_bloqueado_seg >= UMBRAL_CEDER_PASO_SEG:
		_tiempo_bloqueado_seg = 0.0
		necesita_reubicacion.emit(self, _estacion_actual.id)


## Si estamos parados sobre la vía PRINCIPAL de nuestro lado y hay algún
## apartadero libre en nuestro corredor, nos movemos a él (sin tocar
## _idx_objetivo/_direccion: seguimos con el mismo destino, solo cambiamos de
## andén). Usado tanto por un tren ya sin servicio (_esperar_asignacion) como
## por uno que lleva demasiado tiempo bloqueado intentando salir (ver
## _ceder_paso_si_procede) — en ambos casos el objetivo es el mismo: no
## bloquear con nuestra espera el tránsito de los demás por esa vía principal.
func _mover_a_apartadero_si_hay() -> bool:
	if _via_actual < 0 or not (_estacion_actual.es_principal(_via_actual, "A") or _estacion_actual.es_principal(_via_actual, "B")):
		return false
	var libre := _estacion_actual.via_libre_no_principal(id_linea)
	if libre < 0 or libre == _via_actual:
		return false
	_estacion_actual.liberar(_via_actual)
	_estacion_actual.ocupar(libre, self)
	_via_actual = libre
	queue_redraw()
	return true


## Ver _mover_a_apartadero_si_hay(): la misma maniobra, pero solo cuando
## llevamos ya UMBRAL_CEDER_PASO_SEG intentando partir sin éxito (un tren que
## simplemente está en su parada comercial normal no debe apartarse por
## sistema: solo el que lleva "demasiado" bloqueado, para no bloquear
## indefinidamente el tránsito de los demás por su vía principal).
func _ceder_paso_si_procede() -> void:
	if _estacion_actual == null or _tiempo_bloqueado_seg < UMBRAL_CEDER_PASO_SEG:
		return
	if _mover_a_apartadero_si_hay():
		_tiempo_bloqueado_seg = 0.0


## Traduce un sentido de movimiento (+1 hacia el final de la ruta, -1 hacia el
## inicio) a "A"/"B" según qué extremo de la línea sea el lado A.
func _dir_para(dir_mov: float) -> String:
	var hacia_fin_de_ruta := dir_mov > 0.0
	var hacia_lado_a := hacia_fin_de_ruta == _lado_a_es_fin
	return "A" if hacia_lado_a else "B"


## Lado (terminal) hacia el que partirá este tren la próxima vez que arranque.
## Lo usa la interfaz de estación para saber qué semáforo interno mirar y hacia
## qué columna debe apuntar el triángulo del tren.
func direccion_salida() -> String:
	return _dir_salida


## Minutos de retraso respecto a la hora prevista de su última asignación
## (0 si salió puntual o adelantado). Lo dibuja _draw() sobre el tren.
func retraso_seg() -> float:
	return _retraso_seg


## Permite al jugador invertir la dirección de salida de un tren PARADO que
## tiene un servicio activo en curso (p. ej. tras reasignar la vía principal
## porque su semáforo interno sigue en rojo). El tren pasa a dirigirse hacia
## la estación de la que partió este servicio (intercambiamos origen/destino
## asignados, así siempre tiene un destino válido). No aplica a un tren
## aparcado SIN asignación: aún no tiene ningún sentido que invertir.
func invertir_sentido_salida() -> void:
	if _estado != Estado.PARADO or _via_actual < 0:
		return
	if _idx_objetivo == _idx_destino_final:
		return
	var anterior_origen := _idx_origen_servicio
	_idx_origen_servicio = _idx_destino_final
	_idx_destino_final = anterior_origen
	_direccion = -_direccion
	_dir_salida = _dir_para(_direccion)
	queue_redraw()


## Para el panel de estación: hacia qué estación nos dirigimos ahora mismo.
func proxima_estacion_id() -> String:
	if _idx_objetivo >= 0 and _idx_objetivo < _ruta_ids.size():
		return _ruta_ids[_idx_objetivo]
	return ""


## Para ScheduleManager (dispersión de flota a última hora, ver
## _on_disponible_en_terminal): ¿estamos parados sobre la vía PRINCIPAL de
## nuestra estación actual? Si es así y ya no nos hace falta aquí, es cuando
## hay que apartarnos a otra estación en vez de quedarnos bloqueando el paso.
func esta_en_via_principal() -> bool:
	if _estacion_actual == null or _via_actual < 0:
		return false
	return _estacion_actual.es_principal(_via_actual, "A") or _estacion_actual.es_principal(_via_actual, "B")


## Para "ir al tren" (TrenesPanel): si estamos parados dentro de una estación
## (en cualquier vía, principal o apartadero), el enclavamiento de esa
## estación — para poder abrir su panel automáticamente. Null si circulamos.
func interlock_actual() -> EstacionInterlock:
	return _estacion_actual if _estado == Estado.PARADO else null


## Para el panel de estación: tiempo (en segundos de juego) hasta la próxima parada.
func eta_segundos_juego() -> float:
	if _estado == Estado.PARADO:
		return 0.0
	if _paradas.size() < 2 or _idx_objetivo < 0 or _idx_objetivo >= _paradas.size():
		return -1.0
	var rem := absf(_paradas[_idx_objetivo] - _progreso)
	if _velocidad_actual_px_seg <= 0.0:
		return -1.0
	return rem / _velocidad_actual_px_seg


func _intentar_partir() -> void:
	# Llevamos demasiado tiempo intentando salir sin conseguirlo: si hay un
	# apartadero libre AQUÍ MISMO, nos apartamos a él para dejar la vía
	# principal libre al resto de tráfico de nuestro lado (así un tren
	# retrasado no bloquea indefinidamente a todos los que vienen detrás; ver
	# UMBRAL_CEDER_PASO_SEG). Si no hay apartadero (p. ej. una estación de
	# solo 1-2 vías), esto no hace nada: seguimos esperando como siempre.
	_ceder_paso_si_procede()

	# (a) Semáforo INTERNO de nuestra vía, en el sentido de salida.
	if _estacion_actual != null and _via_actual >= 0:
		if not _estacion_actual.semaforo_verde(_via_actual, _dir_salida):
			return   # interno en rojo: nos quedamos estacionados indefinidamente

	var siguiente := _idx_objetivo + int(_direccion)
	var nuevo := _bloque_entre(_idx_objetivo, siguiente)

	# (b) Tránsito físico del cantón: cuenta siempre, sea cual sea mi destino.
	if nuevo != null and nuevo.ocupado:
		return

	# (b2) Vía única (ver TramoViaUnica.gd): no-op en tramos de vía doble. Si
	# el sentido contrario ya tiene el testigo de este tramo (circulando o con
	# salida ya concedida en otra estación), nos quedamos esperando aunque el
	# cantón esté físicamente libre — así se evita el choque frontal.
	if nuevo != null and not nuevo.via_unica_disponible():
		return

	# (c) Vía de la estación DESTINO libre. Si MI servicio termina justo ahí
	#     (siguiente == mi propio destino final, sea el final de línea de
	#     verdad o un giro corto intermedio), no me importa qué vía en
	#     concreto: cualquiera libre me vale porque no seguiré de largo.
	#     ESTO ES UNA CONDICIÓN DE ESTE TREN, no del tramo: un tren que solo
	#     pasa de largo por esta estación NUNCA se beneficia de esto, aunque
	#     la estación sea terminal de otra línea o de un giro corto de un
	#     tercero — por eso se decide aquí y no en Bloque (que es compartido
	#     entre líneas/servicios y no puede saber quién llega cada vez).
	#     Si no termino aquí, hace falta LA vía principal de mi lado
	#     específicamente; si está ocupada, espero (en la práctica, esto deja
	#     "en rojo" el tramo anterior).
	#
	#     VÍA ÚNICA (ver EstacionInterlock.reservar): no basta con que la vía
	#     esté libre AHORA MISMO — tardamos minutos en llegar, y otro tren
	#     podría "verla" libre en un instante distinto y converger también
	#     hacia ella (deadlock real de vía única, visto en Parets del
	#     Vallès: dos trenes en sentidos opuestos se comprometían cada uno a
	#     una vía que le parecía libre al partir, y al final ninguno cabía).
	#     Exigimos que la vía esté RESERVABLE (libre y no ya prometida a otro
	#     tren de camino) y, si partimos, la reservamos de inmediato: así
	#     ningún otro puede comprometerse a la misma vía mientras viajamos.
	var sid_destino := ""
	if siguiente >= 0 and siguiente < _ruta_ids.size():
		sid_destino = _ruta_ids[siguiente]
	var inter_destino := _interlocks.get(sid_destino, null) as EstacionInterlock
	var via_a_reservar := -1
	if inter_destino != null:
		var es_via_unica := nuevo != null and nuevo.es_via_unica()
		if siguiente == _idx_destino_final:
			if es_via_unica:
				via_a_reservar = inter_destino.primera_via_libre_reservable(id_linea, self)
				if via_a_reservar < 0:
					return
			elif inter_destino.primera_via_libre(id_linea) < 0:
				return
		else:
			var via_d := inter_destino.via_principal(_dir_salida, id_linea)
			if es_via_unica:
				if not inter_destino.via_libre_para_reserva(via_d, self):
					return
				via_a_reservar = via_d
			else:
				var ocup_d: Variant = inter_destino.tren_en(via_d)
				if ocup_d != null and ocup_d != self:
					return

	# Partimos: liberamos la vía de la estación (el cantón exterior de llegada
	# ya se liberó en _llegar_a_estacion) y ocupamos el nuevo cantón de salida.
	if _estacion_actual != null and _via_actual >= 0:
		_estacion_actual.liberar(_via_actual)
	_estacion_actual = null
	_via_actual = -1
	if via_a_reservar >= 0 and inter_destino != null:
		inter_destino.reservar(via_a_reservar, self)
	if nuevo != null:
		nuevo.ocupado = true
		nuevo.reservar_via_unica()
	_bloque_actual = nuevo
	_velocidad_actual_px_seg = _velocidad_tramo(_idx_objetivo, siguiente)
	_idx_objetivo = siguiente
	_estado = Estado.MARCHA
	queue_redraw()


## Segundos->px/seg del tramo entre los índices `a` y `b` (adyacentes),
## usando el tiempo constante de horario_topologia.json y la distancia real
## de la curva entre esos dos puntos.
func _velocidad_tramo(a: int, b: int) -> float:
	var tramo_idx := mini(a, b)
	var duracion := 1.0
	if tramo_idx >= 0 and tramo_idx < _tramos.size():
		duracion = maxf(float(_tramos[tramo_idx]), 1.0)
	var distancia := absf(_paradas[b] - _paradas[a])
	return distancia / duracion


## Primer cantón de la cadena del tramo (a,b) en nuestro sentido — de paso,
## prepara toda la cadena (ver _preparar_cantones) para poder ir cruzando el
## resto de cantones intermedios durante el tránsito, no solo el primero.
func _bloque_entre(a: int, b: int) -> Bloque:
	if a < 0 or b < 0 or a >= _ruta_ids.size() or b >= _ruta_ids.size():
		_cantones_tramo = []
		_canton_offsets = []
		return null
	_preparar_cantones(a, b)
	if _cantones_tramo.is_empty():
		return null
	return _cantones_tramo[0] as Bloque


## Calcula, para el tramo entre los índices adyacentes `a` y `b`, la cadena de
## cantones en NUESTRO sentido (ida si b>a, vuelta si b<a) y el offset de
## progreso donde acaba cada uno (repartidos a partes iguales entre las dos
## estaciones — el tramo entre dos estaciones adyacentes es recto, así que un
## reparto lineal del offset ya da tramos de igual longitud real).
func _preparar_cantones(a: int, b: int) -> void:
	_canton_actual = 0
	_cantones_tramo = []
	_canton_offsets = []
	var tramo_idx := mini(a, b)
	if tramo_idx < 0 or tramo_idx >= _cantones.size():
		return
	var entry: Dictionary = _cantones[tramo_idx]
	var bloques: Array = (entry.get("bloques_ida", []) if b > a else entry.get("bloques_vuelta", [])) as Array
	if bloques.is_empty():
		return
	var origen := _paradas[a]
	var destino := _paradas[b]
	var n := bloques.size()
	for k in n:
		_canton_offsets.append(origen + (destino - origen) * float(k + 1) / float(n))
	_cantones_tramo = bloques


## Distancia de frenado, expresada como el tiempo (de juego) que tardaríamos
## en recorrerla a velocidad de crucero -- así escala sola con la velocidad
## de cada tramo/modelo sin necesidad de una constante en píxeles.
const TIEMPO_FRENADO_SEG := 10.0

## Cuánto nos quedamos ANTES del límite físico del cantón (= la posición real
## del semáforo, ver MapaCatalunya._crear_cantones_tramo) al parar: si
## paráramos con el CENTRO del tren exactamente en el límite, la mitad
## delantera del tren (LARGO/2) quedaría dibujada por encima/más allá del
## semáforo. Medio tren más un pequeño margen visual deja el morro justo
## detrás, nunca encima.
const MARGEN_PARADA_SEMAFORO := LARGO / 2.0 + 1.0

## Punto de parada real ante el semáforo que protege la entrada al cantón
## `_canton_actual + 1` -- el límite físico del cantón actual, retranqueado
## MARGEN_PARADA_SEMAFORO en el sentido CONTRARIO a la marcha.
func _punto_parada_actual() -> float:
	var limite: float = _canton_offsets[_canton_actual]
	return limite - _direccion * MARGEN_PARADA_SEMAFORO


## Velocidad a aplicar ESTE fotograma. Normalmente la de crucero; si el
## PRÓXIMO cantón de la cadena está en rojo y ya estamos dentro de la
## distancia de frenado, decae suavemente con un perfil de deceleración
## constante (v = v0·√(distancia_restante / distancia_frenado)) para que el
## tren llegue frenando, no a golpe de freno, y quede detenido justo detrás
## del semáforo -- el punto EXACTO de parada lo sigue fijando el clamp de
## _actualizar_canton_intermedio (_progreso = _punto_parada_actual()), esto
## solo suaviza la aproximación. Si el cantón se libera antes de llegar,
## recupera la velocidad de crucero de inmediato (sin rampa de aceleración:
## el frenado es lo único que pide el enunciado).
func _velocidad_frenada() -> float:
	if _canton_actual >= _cantones_tramo.size() - 1:
		return _velocidad_actual_px_seg   # último cantón: entra en la estación, no hay semáforo que frenar aquí
	var siguiente_bloque := _cantones_tramo[_canton_actual + 1] as Bloque
	if not siguiente_bloque.en_rojo_para(id_linea):
		return _velocidad_actual_px_seg
	var distancia_restante := absf(_punto_parada_actual() - _progreso)
	var distancia_frenado := _velocidad_actual_px_seg * TIEMPO_FRENADO_SEG
	if distancia_frenado <= 0.0 or distancia_restante >= distancia_frenado:
		return _velocidad_actual_px_seg
	var factor := sqrt(clampf(distancia_restante / distancia_frenado, 0.0, 1.0))
	return _velocidad_actual_px_seg * factor


## Mientras circulamos, comprueba si ya hemos alcanzado el punto de parada
## ante el semáforo que protege el cantón intermedio siguiente: si está
## libre, lo cruzamos sin más (soltamos el anterior y reclamamos el nuevo,
## dejando que _progreso siga avanzando con normalidad); si no, nos quedamos
## parados justo detrás del semáforo (nunca encima) hasta que se libere. El
## ÚLTIMO cantón de la cadena es el que entra en la estación destino — de ese
## ya se encarga _llegar_a_estacion como siempre, así que aquí no tocamos
## nada una vez llegados a él.
func _actualizar_canton_intermedio() -> void:
	if _canton_actual >= _cantones_tramo.size() - 1:
		return   # ya vamos por el último cantón de la cadena: nada más que cruzar
	var punto_parada := _punto_parada_actual()
	var alcanzado := (_direccion > 0.0 and _progreso >= punto_parada) or (_direccion < 0.0 and _progreso <= punto_parada)
	if not alcanzado:
		return
	var siguiente_bloque := _cantones_tramo[_canton_actual + 1] as Bloque
	if siguiente_bloque.en_rojo_para(id_linea):
		_progreso = punto_parada   # nos quedamos justo detrás del semáforo hasta que se libere
		return
	var actual_bloque := _cantones_tramo[_canton_actual] as Bloque
	actual_bloque.ocupado = false
	siguiente_bloque.ocupado = true
	_canton_actual += 1
	_bloque_actual = siguiente_bloque


func _draw() -> void:
	var cuerpo := Rect2(-LARGO / 2.0, -ANCHO / 2.0, LARGO, ANCHO)
	draw_rect(cuerpo, color, true)
	var borde := Color(1, 1, 1, 0.9) if _estado == Estado.PARADO else Color(0, 0, 0, 0.6)
	draw_rect(cuerpo, borde, false, 1.5)
	# El retraso ya no se dibuja fijo aquí: se muestra en un tooltip al pasar
	# el ratón por encima del tren (ver _configurar_hover()/_actualizar_tooltip()).
