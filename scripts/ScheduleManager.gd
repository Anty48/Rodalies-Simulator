class_name ScheduleManager
extends Node
## ════════════════════════════════════════════════════════════════════════════
##  ScheduleManager.gd  —  Motor de horarios: reparte los trenes entre servicios
## ════════════════════════════════════════════════════════════════════════════
##  La flota es FINITA: se crea una vez, al principio del día, y esos mismos
##  trenes van encadenando servicios toda la jornada (nunca se destruyen). Por
##  cada línea mantenemos, POR ESTACIÓN DE ORIGEN:
##    - una cola de servicios pendientes (ordenada por hora de salida), y
##    - una cola de trenes disponibles (los que ya están aparcados ahí).
##  Cada fotograma solo miramos, por estación, si el servicio más antiguo de la
##  cola ya tocaba salir Y si hay algún tren libre — O(líneas × terminales),
##  nunca recorremos todos los servicios del día ni todos los trenes.
##
##  Si a un servicio le toca salir y no hay tren libre en su origen, se queda
##  pendiente (retraso real, no un tren fantasma) hasta que llegue uno — algo
##  que puede pasar si el jugador retiene un tren en ruta y nunca llega a la
##  terminal para cubrir la vuelta.
##
##  FLOTA INICIAL: al arrancar, ningún tren ha "llegado" todavía de ningún
##  sitio, así que hace falta crear de entrada tantos como para no dejar
##  huecos. Lo calculamos con una simulación greedy de un solo día (salidas y
##  llegadas NOMINALES, con los tramos/esperas constantes): cada vez que una
##  salida no tendría, en teoría, ningún tren ya "disponible" por una llegada
##  previa, apuntamos que hace falta uno más ya aparcado en ese origen.
##
##  REPOSICIONAMIENTO: una terminal solo puede aparcar tantos trenes como vías
##  tenga (menos una, que dejamos siempre libre para el tráfico normal). Si la
##  flota que necesita supera esa capacidad, el sobrante (los que menos prisa
##  corren: los de salida más tardía) se busca automáticamente la estación con
##  vías libres MÁS CERCANA en la topología de esa misma línea (ver
##  _buscar_reserva_cercana(), que recorre la ruta hacia los dos lados y compara
##  tiempo de viaje real) y viaja "en vacío" hasta la terminal saturada con la
##  antelación justa para su hora de salida real (tiempo de viaje + margen de
##  giro, no un margen fijo arbitrario) — así se evita el colapso sin inventar
##  trenes de más, sin tocar vías de estaciones que no lo necesitan y sin
##  depender de una tabla de reservas escrita a mano por línea.
## ════════════════════════════════════════════════════════════════════════════

const MODELOS := ["447", "465", "450", "490"]

## Reparte modelos por turno rotatorio (round-robin) como siempre, pero
## saltándose los que declaren "lineas_permitidas" en modelos_trenes.json y no
## incluyan `id_linea` (p. ej. el UT 450 de dos pisos, solo R2/R2_NORD/R2_SUD
## por gálibo/demanda histórica) -- así nunca se asigna fuera de su línea real,
## sin necesidad de tocar la lista MODELOS ni el resto del reparto.
func _elegir_modelo(id_linea: String) -> String:
	for _intento in MODELOS.size():
		var candidato: String = MODELOS[_contador_modelo % MODELOS.size()]
		_contador_modelo += 1
		var lineas_permitidas: Array = Global.get_modelo_tren(candidato).get("lineas_permitidas", [])
		if lineas_permitidas.is_empty() or id_linea in lineas_permitidas:
			return candidato
	return MODELOS[0]   # nunca debería llegar aquí (al menos un modelo sin restricción, "447")
const BUFFER_CAMBIO_SENTIDO_SEG := 5.0 * 60.0   # debe COINCIDIR con Tren.GIRO_MINIMO_SEG

var _capa_trenes: Node2D
var _curvas_por_linea: Dictionary = {}
var _rutas_ids: Dictionary = {}
var _interlocks: Dictionary = {}
var _lado_a_es_fin_por_linea: Dictionary = {}
var _trenes: Array = []             # referencia compartida con MapaCatalunya/Estacion (para ETA)
var _cantones_por_linea: Dictionary = {}   # id_linea -> Array (por tramo: {"bloques_ida":Array[Bloque],"bloques_vuelta":Array[Bloque]})

var _tramos_por_linea: Dictionary = {}     # id_linea -> Array[float] (segundos, constante)
var _esperas_por_linea: Dictionary = {}    # id_linea -> Array[float] (segundos, constante)
var _servicios_por_linea: Dictionary = {}  # id_linea -> Array[{origen, origen_idx, destino_idx, salida_seg}] (ordenado)
var _pendientes: Dictionary = {}           # id_linea -> id_estacion(origen) -> Array[{destino_idx, salida_seg}] (FIFO)
var _disponibles: Dictionary = {}          # id_linea -> id_estacion -> Array[Tren] (FIFO, orden de llegada)
var _reposiciones_pendientes: Array = []   # Array[{"tren":Tren, "idx_destino":int, "hora":float}]
var _contador_modelo: int = 0
var _siguiente_id_tren: int = 1            # para Tren.id_tren, ver _crear_tren_en/_crear_tren_reposicion
var _acumulador_debug_seg: float = 0.0     # segundos REALES (no de juego) desde el último volcado


func configurar(capa_trenes: Node2D, curvas_por_linea: Dictionary, rutas_ids: Dictionary,
		interlocks: Dictionary, lado_a_es_fin_por_linea: Dictionary, trenes: Array, cantones_por_linea: Dictionary) -> void:
	_capa_trenes = capa_trenes
	_curvas_por_linea = curvas_por_linea
	_rutas_ids = rutas_ids
	_interlocks = interlocks
	_lado_a_es_fin_por_linea = lado_a_es_fin_por_linea
	_trenes = trenes
	_cantones_por_linea = cantones_por_linea

	for id_linea in Global.lineas.keys():
		_tramos_por_linea[str(id_linea)] = Global.get_tramos_linea(str(id_linea))
		_esperas_por_linea[str(id_linea)] = Global.get_esperas_linea(str(id_linea))

	_preparar_servicios()
	_generar_flota_inicial()


func _process(delta: float) -> void:
	var ahora := Global.tiempo_juego_seg

	if Global.debug_log_trenes:
		_acumulador_debug_seg += delta
		if _acumulador_debug_seg >= Global.DEBUG_LOG_INTERVALO_SEG:
			_acumulador_debug_seg = 0.0
			_volcar_estado_trenes()

	var i := 0
	while i < _reposiciones_pendientes.size():
		var r := _reposiciones_pendientes[i] as Dictionary
		if float(r["hora"]) <= ahora:
			(r["tren"] as Tren).asignar_reposicionamiento(int(r["idx_destino"]))
			_reposiciones_pendientes.remove_at(i)
		else:
			i += 1

	for id_linea in _pendientes.keys():
		var por_estacion: Dictionary = _pendientes[id_linea]
		var disponibles_linea: Dictionary = _disponibles.get(id_linea, {})
		for id_estacion in por_estacion.keys():
			var cola: Array = por_estacion[id_estacion]
			if cola.is_empty():
				continue
			var siguiente := cola[0] as Dictionary
			if float(siguiente["salida_seg"]) > ahora:
				continue   # aún no toca; al estar ordenada, no hace falta seguir mirando esta estación

			var libres: Array = disponibles_linea.get(id_estacion, [])
			if libres.is_empty():
				continue   # toca salir pero no hay tren libre: queda pendiente (retraso real)

			var tren := libres.pop_front() as Tren
			cola.pop_front()
			tren.asignar_servicio(int(siguiente["destino_idx"]), float(siguiente["salida_seg"]))


# ─────────────────────────────────────────────────────────────────────────────
#  PREPARACIÓN: de horario_servicios.json (ids de estación) a índices de ruta
# ─────────────────────────────────────────────────────────────────────────────

func _preparar_servicios() -> void:
	for id_linea in Global.horario_servicios.keys():
		var linea := str(id_linea)
		var ids: Array = _rutas_ids.get(linea, [])
		var bruta := Global.get_servicios_linea(linea)
		if ids.is_empty() or bruta.is_empty():
			continue   # p. ej. R3: topología cargada pero sin servicios (en obras)

		var indice: Dictionary = {}
		for i in ids.size():
			indice[str(ids[i])] = i

		var procesados: Array = []
		for s in bruta:
			var datos := s as Dictionary
			var origen := str(datos.get("origen", ""))
			var destino := str(datos.get("destino", ""))
			if not indice.has(origen) or not indice.has(destino):
				continue
			procesados.append({
				"origen": origen,
				"origen_idx": int(indice[origen]),
				"destino_idx": int(indice[destino]),
				"salida_seg": _hms_a_seg(str(datos.get("salida", "00:00:00"))),
			})
		procesados.sort_custom(_por_salida_ascendente)
		_servicios_por_linea[linea] = procesados

		# Agrupamos por estación de origen; como `procesados` ya está ordenado
		# por hora, cada sublista sale ordenada también sin necesidad de re-ordenar.
		var por_estacion: Dictionary = {}
		for s in procesados:
			var datos2 := s as Dictionary
			var origen2 := str(datos2["origen"])
			var lista: Array = por_estacion.get(origen2, [])
			lista.append({"destino_idx": int(datos2["destino_idx"]), "salida_seg": float(datos2["salida_seg"])})
			por_estacion[origen2] = lista

		_pendientes[linea] = por_estacion
		_disponibles[linea] = {}


func _por_salida_ascendente(a: Dictionary, b: Dictionary) -> bool:
	return float(a["salida_seg"]) < float(b["salida_seg"])


func _hms_a_seg(hms: String) -> float:
	var partes := hms.split(":")
	if partes.size() < 3:
		return 0.0
	return float(int(partes[0]) * 3600 + int(partes[1]) * 60 + int(partes[2]))


# ─────────────────────────────────────────────────────────────────────────────
#  FLOTA INICIAL (trenes ya aparcados al empezar la jornada)
# ─────────────────────────────────────────────────────────────────────────────

func _generar_flota_inicial() -> void:
	# Primero recogemos TODOS los huecos de TODAS las líneas, agrupados por
	# CLAVE DE CAPACIDAD (estación + corredor, ver _clave_capacidad): varias
	# líneas pueden compartir la misma estación física y por tanto el mismo
	# cupo real de vías (p. ej. Granollers para R2/R2_NORD/R8), pero solo si
	# además comparten CORREDOR — una estación con corredores segregados (p.
	# ej. Barcelona El Clot: R1 en unas vías, R2/R2_NORD en otras) reparte la
	# capacidad de cada corredor por separado, nunca entre ambos.
	var huecos_por_clave: Dictionary = {}   # clave -> Array[{"linea":.., "estacion":.., "salida_seg":..}]
	for id_linea in _servicios_por_linea.keys():
		var linea := str(id_linea)
		for hueco in _calcular_flota_necesaria(linea):
			var datos := hueco as Dictionary
			var estacion := str(datos["estacion"])
			var clave := _clave_capacidad(estacion, linea)
			var lista: Array = huecos_por_clave.get(clave, [])
			lista.append({"linea": linea, "estacion": estacion, "salida_seg": float(datos["salida_seg"])})
			huecos_por_clave[clave] = lista

	# Cuántas vías le quedan libres a cada clave para aparcar trenes: se va
	# consumiendo tanto por sus propios huecos (fase 1) como por el sobrante
	# de OTRAS estaciones/corredores que la usen como reserva cercana (fase 2).
	var capacidad_restante: Dictionary = {}    # clave -> int
	var overflow: Array = []                   # [{"linea":.., "estacion":.., "salida_seg":..}]

	# Fase 1: cada clave reclama PRIMERO su propia capacidad para sus huecos
	# más tempranos (los que menos margen tienen para ir a buscar una reserva).
	# Así el sobrante de una estación nunca le quita a otra la vía que necesita
	# para sus propios trenes, sea cual sea el orden en que las recorramos.
	for clave in huecos_por_clave.keys():
		var lista: Array = huecos_por_clave[clave]
		lista.sort_custom(_por_salida_ascendente)
		var muestra := lista[0] as Dictionary   # misma estación+corredor en toda la lista
		var capacidad := _capacidad_directa(str(muestra["estacion"]), str(muestra["linea"]))
		capacidad_restante[clave] = capacidad
		for i in lista.size():
			var datos := lista[i] as Dictionary
			var linea := str(datos["linea"])
			var estacion := str(datos["estacion"])
			if i < capacidad:
				capacidad_restante[clave] = int(capacidad_restante[clave]) - 1
				_crear_tren_en(linea, estacion)
			else:
				overflow.append({"linea": linea, "estacion": estacion, "salida_seg": float(datos["salida_seg"])})

	# Fase 2: el sobrante (ordenado cronológicamente entre TODAS las estaciones,
	# así el que menos margen real tiene reclama antes el hueco de reserva que
	# le convenga) busca automáticamente la estación con vías libres más
	# cercana, EN LA TOPOLOGÍA DE SU PROPIA LÍNEA (un tren no puede circular por
	# líneas que no recorre), y viaja hacia allí en vacío.
	overflow.sort_custom(_por_salida_ascendente)
	for datos in overflow:
		var linea := str((datos as Dictionary)["linea"])
		var estacion := str((datos as Dictionary)["estacion"])
		var salida_seg := float((datos as Dictionary)["salida_seg"])
		var reserva := _buscar_reserva_cercana(linea, estacion, capacidad_restante)
		if reserva != "":
			var clave_reserva := _clave_capacidad(reserva, linea)
			capacidad_restante[clave_reserva] = _capacidad_libre(reserva, linea, capacidad_restante) - 1
			_crear_tren_reposicion(linea, reserva, estacion, salida_seg)
		else:
			_crear_tren_en(linea, estacion)   # ninguna estación vecina tenía sitio: mejor aparcarlo aqui que no crearlo


## Identifica de forma única un "cupo de aparcamiento": estación + corredor
## (ver EstacionInterlock). Dos líneas del MISMO corredor en la misma estación
## comparten clave (y por tanto cupo real); dos líneas de corredores distintos
## en la misma estación física NUNCA la comparten, aunque el id de estación
## sea el mismo — sus vías son físicamente independientes.
func _clave_capacidad(id_estacion: String, id_linea: String) -> String:
	var interlock := _interlocks.get(id_estacion, null) as EstacionInterlock
	if interlock == null:
		return id_estacion
	return "%s#%d" % [id_estacion, interlock.corredor_de_linea(id_linea)]


## Cuántas vías del corredor de `id_linea` en `id_estacion` sirven para
## aparcar un tren que solo espera su horario: NUNCA una principal
## (bloquearía el cantón de ese sentido para cualquier otro tren, como pasaba
## en Montmeló), así que es directamente su número de apartaderos reales. Un
## corredor de 1 o 2 vías (todas principales, p. ej. Montmeló, o cada mitad
## segregada de El Clot) da 0: no sirve para este tipo de espera, ni como
## estación propia ni como reserva cercana de otra.
func _capacidad_directa(id_estacion: String, id_linea: String) -> int:
	var interlock := _interlocks.get(id_estacion, null) as EstacionInterlock
	if interlock == null:
		return 0
	return interlock.num_vias_no_principales(id_linea)


## Vías libres que le quedan AHORA al cupo (estación+corredor) de `id_linea`
## en `id_estacion` (se inicializa perezosamente a su capacidad directa la
## primera vez que se consulta).
func _capacidad_libre(id_estacion: String, id_linea: String, capacidad_restante: Dictionary) -> int:
	var clave := _clave_capacidad(id_estacion, id_linea)
	if not capacidad_restante.has(clave):
		capacidad_restante[clave] = _capacidad_directa(id_estacion, id_linea)
	return int(capacidad_restante[clave])


## Busca, recorriendo la topología de `linea` hacia los dos lados desde
## `id_estacion_saturada`, la estación con vías libres MÁS CERCANA por tiempo
## de viaje real (no por número de paradas). Un tren solo puede reposicionarse
## por SU PROPIA línea (viaja sobre su curva), así que la búsqueda no sale de
## `_rutas_ids[linea]`. Devuelve "" si ninguna vecina tiene sitio.
func _buscar_reserva_cercana(linea: String, id_estacion_saturada: String, capacidad_restante: Dictionary) -> String:
	var ids: Array = _rutas_ids.get(linea, [])
	var tramos: Array = _tramos_por_linea.get(linea, [])
	var idx_sat := -1
	for i in ids.size():
		if str(ids[i]) == id_estacion_saturada:
			idx_sat = i
			break
	if idx_sat < 0:
		return ""

	var cand_izq := ""
	var tiempo_izq := INF
	var t := 0.0
	var i := idx_sat
	while i > 0:
		t += float(tramos[i - 1])
		i -= 1
		if _capacidad_libre(str(ids[i]), linea, capacidad_restante) > 0:
			cand_izq = str(ids[i])
			tiempo_izq = t
			break

	var cand_der := ""
	var tiempo_der := INF
	t = 0.0
	var j := idx_sat
	while j < ids.size() - 1:
		t += float(tramos[j])
		j += 1
		if _capacidad_libre(str(ids[j]), linea, capacidad_restante) > 0:
			cand_der = str(ids[j])
			tiempo_der = t
			break

	if cand_izq == "":
		return cand_der
	if cand_der == "":
		return cand_izq
	return cand_izq if tiempo_izq <= tiempo_der else cand_der


## Como _buscar_reserva_cercana(), pero para DISPERSAR flota fuera de servicio
## en lugar de concentrarla al principio del día (ver _on_disponible_en_terminal):
## recorre la topología de `linea` hacia los dos lados desde la estación
## saturada y compara tiempo de viaje real, igual que aquella — la diferencia
## es que aquí consultamos la ocupación EN VIVO de cada estación candidata
## (_tiene_apartadero_libre) en vez de un presupuesto de capacidad calculado
## una sola vez al arrancar: esto se llama en cualquier momento del día, así
## que el "libre" tiene que ser el de ESE instante. Devuelve "" si ninguna
## vecina tiene sitio ahora mismo.
func _buscar_deposito_cercano(linea: String, id_estacion_saturada: String) -> String:
	var ids: Array = _rutas_ids.get(linea, [])
	var tramos: Array = _tramos_por_linea.get(linea, [])
	var idx_sat := -1
	for i in ids.size():
		if str(ids[i]) == id_estacion_saturada:
			idx_sat = i
			break
	if idx_sat < 0:
		return ""

	var cand_izq := ""
	var tiempo_izq := INF
	var t := 0.0
	var i := idx_sat
	while i > 0:
		t += float(tramos[i - 1])
		i -= 1
		if _tiene_apartadero_libre(str(ids[i]), linea):
			cand_izq = str(ids[i])
			tiempo_izq = t
			break

	var cand_der := ""
	var tiempo_der := INF
	t = 0.0
	var j := idx_sat
	while j < ids.size() - 1:
		t += float(tramos[j])
		j += 1
		if _tiene_apartadero_libre(str(ids[j]), linea):
			cand_der = str(ids[j])
			tiempo_der = t
			break

	if cand_izq == "":
		return cand_der
	if cand_der == "":
		return cand_izq
	return cand_izq if tiempo_izq <= tiempo_der else cand_der


func _tiene_apartadero_libre(id_estacion: String, linea: String) -> bool:
	var interlock := _interlocks.get(id_estacion, null) as EstacionInterlock
	if interlock == null:
		return false
	return interlock.via_libre_no_principal(linea) >= 0


## Tiempo de viaje NOMINAL (constante) entre dos índices de la ruta, sumando
## los tramos que hay entre ellos (en cualquier sentido).
func _tiempo_viaje_entre(linea: String, idx_a: int, idx_b: int) -> float:
	var tramos: Array = _tramos_por_linea.get(linea, [])
	var i0 := mini(idx_a, idx_b)
	var i1 := maxi(idx_a, idx_b)
	var total := 0.0
	for i in range(i0, i1):
		if i < tramos.size():
			total += float(tramos[i])
	return total


## Huecos de flota de una línea: momentos del día en los que, según una
## simulación de un solo día (salidas y llegadas NOMINALES, con los tramos y
## esperas constantes), ningún tren estaría ya "disponible" por una llegada
## previa. Devuelve, en orden cronológico, {"estacion":.., "salida_seg":..}
## por cada hueco — solo para dimensionar la flota al arrancar; a partir de
## ahí el reparto real lo hace _process() en vivo.
func _calcular_flota_necesaria(linea: String) -> Array:
	var lista: Array = _servicios_por_linea.get(linea, [])
	var ids: Array = _rutas_ids.get(linea, [])
	var tramos: Array = _tramos_por_linea.get(linea, [])
	var esperas: Array = _esperas_por_linea.get(linea, [])

	var eventos: Array = []
	for idx in lista.size():
		var s: Dictionary = lista[idx]
		var salida: float = float(s["salida_seg"])
		eventos.append({"t": salida, "tipo": 0, "idx": idx})
		eventos.append({"t": salida + _duracion_servicio(s, tramos, esperas), "tipo": 1, "idx": idx})
	eventos.sort_custom(_por_tiempo_llegadas_primero)

	var huecos: Array = []
	var disponibles_sim: Dictionary = {}   # id_estacion -> Array[float] (horas de llegada+margen, ascendente)
	for ev in eventos:
		var idx: int = int(ev["idx"])
		var s: Dictionary = lista[idx]
		if int(ev["tipo"]) == 1:
			var sid_llegada := str(ids[int(s["destino_idx"])])
			var cola: Array = disponibles_sim.get(sid_llegada, [])
			cola.append(float(ev["t"]) + BUFFER_CAMBIO_SENTIDO_SEG)
			cola.sort()
			disponibles_sim[sid_llegada] = cola
		else:
			var sid_salida := str(s["origen"])
			var cola2: Array = disponibles_sim.get(sid_salida, [])
			if cola2.size() > 0 and float(cola2[0]) <= float(ev["t"]):
				cola2.pop_front()
				disponibles_sim[sid_salida] = cola2
			else:
				huecos.append({"estacion": sid_salida, "salida_seg": float(ev["t"])})
	return huecos


func _por_tiempo_llegadas_primero(a: Dictionary, b: Dictionary) -> bool:
	var ta: float = float(a["t"])
	var tb: float = float(b["t"])
	if ta == tb:
		return int(a["tipo"]) > int(b["tipo"])   # llegada (1) antes que salida (0) en un empate
	return ta < tb


## Duración NOMINAL del servicio (tramos + esperas intermedias constantes),
## solo para decidir cuántos trenes hacen falta ya aparcados al principio.
## El tren, una vez en marcha, puede tardar más si un semáforo lo retiene.
func _duracion_servicio(s: Dictionary, tramos: Array, esperas: Array) -> float:
	var i0 := mini(int(s["origen_idx"]), int(s["destino_idx"]))
	var i1 := maxi(int(s["origen_idx"]), int(s["destino_idx"]))
	var total := 0.0
	for i in range(i0, i1):
		if i < tramos.size():
			total += float(tramos[i])
	for i in range(i0 + 1, i1):
		if i < esperas.size():
			total += float(esperas[i])
	return total


# ─────────────────────────────────────────────────────────────────────────────
#  CREACIÓN DE TRENES Y REGISTRO DE DISPONIBILIDAD
# ─────────────────────────────────────────────────────────────────────────────

func _crear_tren_en(id_linea: String, id_estacion: String) -> void:
	var curva := _curvas_por_linea.get(id_linea, null) as Curve2D
	var ids: Array = _rutas_ids.get(id_linea, [])
	if curva == null or ids.is_empty():
		return
	var idx := -1
	for i in ids.size():
		if str(ids[i]) == id_estacion:
			idx = i
			break
	if idx < 0:
		return

	var tren := Tren.new()
	_capa_trenes.add_child(tren)
	tren.id_tren = "T%03d" % _siguiente_id_tren
	_siguiente_id_tren += 1
	var id_modelo: String = _elegir_modelo(id_linea)
	tren.configurar(id_linea, id_modelo, curva, ids,
		_tramos_por_linea.get(id_linea, []), _esperas_por_linea.get(id_linea, []),
		_interlocks, bool(_lado_a_es_fin_por_linea.get(id_linea, true)),
		_cantones_por_linea.get(id_linea, []))
	tren.disponible_en_terminal.connect(_on_disponible_en_terminal)
	tren.necesita_reubicacion.connect(_on_necesita_reubicacion)
	tren.nacer_en_estacion(idx)
	_trenes.append(tren)


## Crea un tren aparcado en `id_estacion_origen` (una reserva con vías libres)
## y programa su viaje en vacío hasta `id_estacion_destino` (la terminal
## saturada), con la antelación justa (tiempo de viaje real) antes de que haga falta ahí.
func _crear_tren_reposicion(id_linea: String, id_estacion_origen: String, id_estacion_destino: String, hora_necesaria_seg: float) -> void:
	var curva := _curvas_por_linea.get(id_linea, null) as Curve2D
	var ids: Array = _rutas_ids.get(id_linea, [])
	if curva == null or ids.is_empty():
		return
	var idx_origen := -1
	var idx_destino := -1
	for i in ids.size():
		if str(ids[i]) == id_estacion_origen:
			idx_origen = i
		elif str(ids[i]) == id_estacion_destino:
			idx_destino = i
	if idx_origen < 0 or idx_destino < 0:
		return

	var tren := Tren.new()
	_capa_trenes.add_child(tren)
	tren.id_tren = "T%03d" % _siguiente_id_tren
	_siguiente_id_tren += 1
	var id_modelo: String = _elegir_modelo(id_linea)
	tren.configurar(id_linea, id_modelo, curva, ids,
		_tramos_por_linea.get(id_linea, []), _esperas_por_linea.get(id_linea, []),
		_interlocks, bool(_lado_a_es_fin_por_linea.get(id_linea, true)),
		_cantones_por_linea.get(id_linea, []))
	tren.disponible_en_terminal.connect(_on_disponible_en_terminal)
	tren.necesita_reubicacion.connect(_on_necesita_reubicacion)
	tren.nacer_en_estacion(idx_origen)
	_trenes.append(tren)

	# Antelación justa: el tiempo de viaje REAL entre la reserva y la terminal
	# saturada (tramos constantes de horario_topologia.json) más el mismo
	# margen mínimo de giro que usa cualquier llegada normal — nunca un margen
	# fijo arbitrario, que podía quedarse corto (y hacer que el reposicionamiento
	# llegara ya tarde, contagiando el retraso al primer servicio comercial del día).
	var duracion := _tiempo_viaje_entre(id_linea, idx_origen, idx_destino)
	var hora_salida := maxf(Global.tiempo_juego_seg, hora_necesaria_seg - duracion - BUFFER_CAMBIO_SENTIDO_SEG)
	_reposiciones_pendientes.append({"tren": tren, "idx_destino": idx_destino, "hora": hora_salida})


## Volcado periódico y desactivable (Global.debug_log_trenes) del estado de
## todos los trenes: línea, dónde están y su retraso. Pensado para depurar el
## reparto de horarios y la señalización sin depender de capturas de pantalla.
func _volcar_estado_trenes() -> void:
	print("── [DEBUG %s] %d trenes ──" % [Global.hora_actual(), _trenes.size()])
	for t in _trenes:
		var tren := t as Tren
		if tren == null:
			continue
		var info := tren.info_debug()
		print("  %s %s | %s -> %s | retraso %ds" % [
			str(info["id_tren"]), str(info["linea"]), str(info["ubicacion"]), str(info["destino"]), int(float(info["retraso_seg"]))
		])


func _on_disponible_en_terminal(tren: Tren, id_estacion: String) -> void:
	var linea := tren.id_linea

	# Dispersión de fin de servicio (evita el colapso nocturno en terminales
	# como L'Hospitalet, ver cabecera): si a esta estación ya no le queda
	# NINGÚN servicio pendiente para esta línea en lo que resta de jornada —
	# es decir, este tren ha terminado de verdad aquí, no va a hacer falta de
	# nuevo — y encima se ha quedado sin apartadero (aparcado en la vía
	# principal), no lo dejamos bloqueando el paso: lo mandamos "en vacío" al
	# apartadero libre más cercano de su propia línea. Si SÍ le queda un
	# apartadero libre aquí mismo, se queda (esa es la excepción: los últimos
	# trenes de la noche pueden dormir en su propia terminal si hay sitio).
	if tren.esta_en_via_principal():
		var pendientes_linea: Dictionary = _pendientes.get(linea, {})
		var cola: Array = pendientes_linea.get(id_estacion, [])
		if cola.is_empty():
			var reserva := _buscar_deposito_cercano(linea, id_estacion)
			if reserva != "" and reserva != id_estacion:
				var ids: Array = _rutas_ids.get(linea, [])
				var idx_destino := ids.find(reserva)
				if idx_destino >= 0:
					tren.asignar_reposicionamiento(idx_destino)
					return   # ya no está "disponible aquí": se ha ido de camino a otra estación
			else:
				push_warning("[ScheduleManager] %s sin apartadero en %s ni en ninguna estación vecina para dispersar: se queda en la vía principal." % [tren.id_tren, id_estacion])

	var mapa: Dictionary = _disponibles.get(linea, {})
	var lista: Array = mapa.get(id_estacion, [])
	lista.append(tren)
	mapa[id_estacion] = lista
	_disponibles[linea] = mapa


## El tren lleva ya UMBRAL_CEDER_PASO_SEG aparcado en su vía PRINCIPAL sin
## conseguir apartadero propio ni servicio asignado (ver Tren._esperar_asignacion):
## seguir así bloquea la entrada de toda la línea a esta estación. Reintentamos
## la MISMA búsqueda de depósito cercano que la dispersión nocturna (ver
## _on_disponible_en_terminal / _buscar_deposito_cercano), pero en cualquier
## momento del día y aunque a esta estación SÍ le queden servicios pendientes
## hoy: si de verdad hace falta aquí, sencillamente no habrá tren disponible
## cuando le toque salir (retraso real), en vez de bloquear a todo el mundo
## mientras tanto. Si no hay ningún depósito libre ahora mismo, no hacemos
## nada: Tren.gd reintentará pasado el mismo umbral.
func _on_necesita_reubicacion(tren: Tren, id_estacion: String) -> void:
	var linea := tren.id_linea
	var reserva := _buscar_deposito_cercano(linea, id_estacion)
	if reserva == "" or reserva == id_estacion:
		return
	var ids: Array = _rutas_ids.get(linea, [])
	var idx_destino := ids.find(reserva)
	if idx_destino < 0:
		return
	var mapa2: Dictionary = _disponibles.get(linea, {})
	var lista2: Array = mapa2.get(id_estacion, [])
	lista2.erase(tren)
	mapa2[id_estacion] = lista2
	_disponibles[linea] = mapa2
	tren.asignar_reposicionamiento(idx_destino)
