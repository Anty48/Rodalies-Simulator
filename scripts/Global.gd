extends Node
## ════════════════════════════════════════════════════════════════════════════
##  Global.gd  —  SINGLETON / AUTOLOAD del Simulador de Rodalies de Catalunya
## ════════════════════════════════════════════════════════════════════════════
##
##  ¿QUÉ ES UN AUTOLOAD?
##  Un Autoload (o "Singleton") es un nodo que Godot crea automáticamente al
##  arrancar el juego y mantiene vivo durante TODA la partida. Existe una única
##  instancia y es accesible desde cualquier otro script con solo escribir su
##  nombre. Por ejemplo, desde el script de un tren podrás hacer:
##        Global.hora_actual()
##        Global.proyectar_estacion("SANTS")
##
##  Si vienes de Python, piénsalo como un módulo importado globalmente
##  (`import config`) combinado con un objeto Singleton compartido por toda la
##  aplicación.
##
##  RESPONSABILIDADES DE ESTE SCRIPT (es el "cerebro" del simulador):
##    1) Cargar TODOS los datos externos en formato JSON  ->  diseño Data-Driven.
##    2) Llevar el reloj interno del juego (de 05:00 a 00:00).
##    3) Controlar el multiplicador de velocidad (Pausa, x0.05, x0.5, x20, x2, x5).
##    4) Calcular el KPI objetivo: minutos de retraso × pasajeros afectados.
##    5) Proyectar coordenadas geográficas (lat/lon) a píxeles de pantalla.
## ════════════════════════════════════════════════════════════════════════════


# ─────────────────────────────────────────────────────────────────────────────
#  SEÑALES (signals)
#  Una señal es un "aviso" que este nodo lanza para que otros reaccionen sin
#  quedar acoplados (es el patrón Observer / publish-subscribe). Por ejemplo,
#  la interfaz se "suscribe" a `tiempo_actualizado` para repintar el reloj.
# ─────────────────────────────────────────────────────────────────────────────
signal tiempo_actualizado(hora_texto: String, tiempo_segundos: float)
signal velocidad_cambiada(multiplicador: float)
signal kpi_actualizado(kpi: float)
signal jornada_finalizada()
signal datos_cargados()


# ─────────────────────────────────────────────────────────────────────────────
#  CONSTANTES DE CONFIGURACIÓN
#  Cambia estos valores para "afinar" el simulador SIN tocar la lógica.
# ─────────────────────────────────────────────────────────────────────────────

## Rutas de los archivos de datos. Si los mueves de carpeta, actualízalas aquí.
const RUTA_ESTACIONES   := "res://datos/estaciones.json"
const RUTA_MODELOS      := "res://datos/modelos_trenes.json"
const RUTA_LINEAS       := "res://datos/lineas.json"
const RUTA_INCIDENCIAS  := "res://datos/incidencias.json"
const RUTA_HORARIO_TOPOLOGIA := "res://datos/horario_topologia.json"
const RUTA_HORARIO_SERVICIOS := "res://datos/horario_servicios.json"

## Horario de servicio (en segundos contados desde la medianoche).
const HORA_INICIO_SEG := 5 * 3600     # 05:00
const HORA_FIN_SEG    := 24 * 3600    # 00:00 (medianoche)

## Perfil de demanda de pasajeros a lo largo del día: pares [segundos, factor]
## que se interpolan linealmente entre sí (ver _actualizar_multiplicador_demanda).
## Apenas hay generación nada más abrir o justo antes de cerrar el servicio, dos
## puntas claras (entrada ~8h, salida ~18:30h) y un valle moderado a mediodía —
## el mismo patrón de cualquier cercanías real, no un flujo constante todo el día.
const PERFIL_DEMANDA_DIA: Array = [
	[5.0 * 3600.0, 0.05],
	[6.0 * 3600.0, 0.3],
	[7.0 * 3600.0, 0.9],
	[8.0 * 3600.0, 1.0],
	[9.0 * 3600.0, 0.8],
	[9.5 * 3600.0, 0.4],
	[12.0 * 3600.0, 0.35],
	[15.0 * 3600.0, 0.35],
	[17.0 * 3600.0, 0.6],
	[18.0 * 3600.0, 0.9],
	[18.5 * 3600.0, 1.0],
	[19.5 * 3600.0, 0.7],
	[21.0 * 3600.0, 0.3],
	[22.5 * 3600.0, 0.15],
	[24.0 * 3600.0, 0.03],
]

## Cuántos SEGUNDOS DE JUEGO pasan por cada SEGUNDO REAL cuando el multiplicador
## de VELOCIDADES vale 1.0. No se toca directamente: las etiquetas de velocidad
## están pensadas para que el juego avance a un múltiplo EXACTO del tiempo real
## (ver VELOCIDADES más abajo), así que si cambias este valor hay que recalcular
## también los multiplicadores para que sigan dando esas proporciones.
const FACTOR_BASE_TIEMPO := 30.0

## Velocidades disponibles. La clave es la etiqueta que ve la jugadora,
## expresada en veces la VELOCIDAD BASE DEL JUEGO (no el tiempo real): "X1" es
## esa base (20x tiempo real, la que se usaba antes por defecto y con la que
## arrancamos siempre), así que "X0.05" es 20 veces más lenta que la base (y
## coincide con volver al tiempo real, 1:1) y "X25" es 25 veces más rápida que
## la base. El valor es el multiplicador que hay que aplicar aTodo verificado todo funciona.
## FACTOR_BASE_TIEMPO para conseguir esa proporción respecto al tiempo real.
const VELOCIDADES := {
	"PAUSA": 0.0,
	"X0.05": 1.0 / FACTOR_BASE_TIEMPO,     #  1x tiempo real =  x0.05 base ( 1/20 de la base)
	"X0.5":  10.0 / FACTOR_BASE_TIEMPO,    # 10x tiempo real =  x0.5  base (10/20 de la base)
	"X1":    20.0 / FACTOR_BASE_TIEMPO,    # 20x tiempo real =  x1    base (velocidad de referencia/por defecto)
	"X2":    40.0 / FACTOR_BASE_TIEMPO,    # 40x tiempo real =  x2    base
	"X5":   100.0 / FACTOR_BASE_TIEMPO,    #100x tiempo real =  x5    base
	"X25":  500.0 / FACTOR_BASE_TIEMPO,    #500x tiempo real =  x25   base (para pruebas masivas)
}

## Área (en píxeles) sobre la que se dibuja el mapa por defecto, y su margen.
## La escena del mapa podrá recalcular la proyección con su tamaño real llamando
## a `preparar_proyeccion()`.
const AREA_MAPA_DEFECTO := Vector2(1920, 1080)
const MARGEN_MAPA_PX := 80.0


# ─────────────────────────────────────────────────────────────────────────────
#  ESTADO DEL JUEGO (variables que cambian en tiempo de ejecución)
# ─────────────────────────────────────────────────────────────────────────────

# --- Datos cargados desde los JSON ---
var modelos_trenes: Dictionary = {}      # { "447": {...}, ... }
var lineas: Dictionary = {}              # { "R1": {...}, ... }
var incidencias: Dictionary = {}         # { "averia_puertas": {...}, ... }
var estaciones: Array = []               # [ {...}, {...} ]  (orden del JSON)
var estaciones_por_id: Dictionary = {}   # { "SANTS": {...} }  (acceso O(1))

# horario_topologia: { "R1": { "estaciones": [{"id":.., "espera_seg":..}, ...], "tramos": [seg, ...] }, ... }
# horario_servicios: { "R1": [ {"origen":.., "destino":.., "salida":"HH:MM:SS"}, ... ], ... }
# Generados por extraer_horarios_pdf.py + generar_horario_oficial.py a partir
# de los PDF oficiales de Rodalies (R1.pdf..R8.pdf); el juego NUNCA lee los
# PDF ni horarios_oficiales.json directamente, solo estos dos.
var horario_topologia: Dictionary = {}
var horario_servicios: Dictionary = {}

# --- Reloj ---
var tiempo_juego_seg: float = HORA_INICIO_SEG
var jornada_terminada: bool = false
var _ultimo_minuto_emitido: int = -1

## Factor de demanda de pasajeros AHORA MISMO (0..1, ver PERFIL_DEMANDA_DIA),
## recalculado una vez por fotograma en _process() y leído por Estacion.gd —
## así las ~123 estaciones no repiten cada una su propia interpolación sobre
## la misma tabla y el mismo reloj.
var multiplicador_demanda_pasajeros: float = 0.05

# --- Velocidad ---
var multiplicador_velocidad: float = float(VELOCIDADES["X1"])    # velocidad de referencia por defecto
var _velocidad_previa: float = float(VELOCIDADES["X1"])          # recuerda la velocidad antes de pausar

# --- Depuración ---
## Booleano "interruptor" para el volcado periódico por consola del estado de
## todos los trenes (línea, estación/cantón y retraso). Desactivado por
## defecto: cámbialo a `true` aquí (o desde el depurador/consola remota) para
## activar el volcado sin tocar más código. Lo consume ScheduleManager._process().
var debug_log_trenes: bool = false
const DEBUG_LOG_INTERVALO_SEG := 5.0

# --- KPI (objetivo a MINIMIZAR) ---
# El KPI es "minutos de retraso × pasajeros afectados"  =>  pasajeros·minuto.
# Es la métrica clásica de "pasajeros-minuto de retraso" usada en transporte.
var total_minutos_retraso: float = 0.0
var total_pasajeros_afectados: int = 0
var kpi_pasajeros_minuto: float = 0.0

# --- Variables internas de la proyección geográfica (se rellenan al cargar) ---
var _proy_lon_min: float = 0.0
var _proy_lat_max: float = 0.0
var _proy_cos_lat: float = 1.0
var _proy_escala: float = 1.0
var _proy_offset: Vector2 = Vector2.ZERO


# ═════════════════════════════════════════════════════════════════════════════
#  CICLO DE VIDA DE GODOT
# ═════════════════════════════════════════════════════════════════════════════

func _ready() -> void:
	# Se ejecuta UNA sola vez al arrancar el juego (por ser un Autoload).
	print("[Global] Iniciando simulador Rodalies…")
	cargar_todos_los_datos()
	preparar_proyeccion(AREA_MAPA_DEFECTO, MARGEN_MAPA_PX)
	datos_cargados.emit()
	print("[Global] Datos listos. Estaciones cargadas: %d" % estaciones.size())


func _process(delta: float) -> void:
	# `delta` son los SEGUNDOS REALES transcurridos desde el fotograma anterior.
	# Avanzamos el reloj solo si NO estamos en pausa ni ha terminado la jornada.
	if multiplicador_velocidad <= 0.0 or jornada_terminada:
		return

	tiempo_juego_seg += delta * FACTOR_BASE_TIEMPO * multiplicador_velocidad

	# ¿Hemos llegado al final del servicio (00:00)?
	if tiempo_juego_seg >= HORA_FIN_SEG:
		tiempo_juego_seg = HORA_FIN_SEG
		jornada_terminada = true
		jornada_finalizada.emit()

	multiplicador_demanda_pasajeros = _calcular_multiplicador_demanda(tiempo_juego_seg)

	# Emitimos la señal del reloj SOLO cuando cambia el minuto que se muestra,
	# así no saturamos a la interfaz con un aviso en cada fotograma.
	var minuto_actual := int(tiempo_juego_seg / 60.0)
	if minuto_actual != _ultimo_minuto_emitido:
		_ultimo_minuto_emitido = minuto_actual
		tiempo_actualizado.emit(hora_actual(), tiempo_juego_seg)


## Interpola PERFIL_DEMANDA_DIA linealmente en `segundos` (fuera de rango se
## queda con el valor del extremo más cercano).
func _calcular_multiplicador_demanda(segundos: float) -> float:
	var perfil := PERFIL_DEMANDA_DIA
	if segundos <= float(perfil[0][0]):
		return float(perfil[0][1])
	for i in range(perfil.size() - 1):
		var t0 := float(perfil[i][0])
		var t1 := float(perfil[i + 1][0])
		if segundos <= t1:
			var m0 := float(perfil[i][1])
			var m1 := float(perfil[i + 1][1])
			var frac := (segundos - t0) / (t1 - t0) if t1 > t0 else 0.0
			return lerpf(m0, m1, frac)
	return float(perfil[perfil.size() - 1][1])


# ═════════════════════════════════════════════════════════════════════════════
#  CARGA DE DATOS  (el corazón del diseño DATA-DRIVEN)
# ═════════════════════════════════════════════════════════════════════════════

func cargar_todos_los_datos() -> void:
	var d_modelos = _cargar_json(RUTA_MODELOS)
	var d_lineas  = _cargar_json(RUTA_LINEAS)
	var d_incid   = _cargar_json(RUTA_INCIDENCIAS)
	var d_estac   = _cargar_json(RUTA_ESTACIONES)
	var d_topologia = _cargar_json(RUTA_HORARIO_TOPOLOGIA)
	var d_servicios = _cargar_json(RUTA_HORARIO_SERVICIOS)

	# Comprobamos el tipo por seguridad (por si falta un archivo o está mal).
	modelos_trenes     = d_modelos    if d_modelos    is Dictionary else {}
	lineas             = d_lineas     if d_lineas     is Dictionary else {}
	incidencias        = d_incid      if d_incid      is Dictionary else {}
	estaciones         = d_estac      if d_estac      is Array      else []
	horario_topologia  = d_topologia  if d_topologia  is Dictionary else {}
	horario_servicios  = d_servicios  if d_servicios  is Dictionary else {}

	# Construimos un índice por `id` para localizar estaciones al instante.
	estaciones_por_id.clear()
	for est in estaciones:
		estaciones_por_id[est["id"]] = est


## Lee un archivo JSON y devuelve su contenido ya convertido a tipos de Godot
## (Dictionary / Array). Devuelve un contenedor vacío si algo falla.
func _cargar_json(ruta: String) -> Variant:
	if not FileAccess.file_exists(ruta):
		push_error("[Global] No se encontró el archivo: " + ruta)
		return {}

	var texto := FileAccess.get_file_as_string(ruta)
	var lector := JSON.new()
	var error := lector.parse(texto)
	if error != OK:
		push_error("[Global] Error de JSON en '%s' (línea %d): %s" % [
			ruta, lector.get_error_line(), lector.get_error_message()
		])
		return {}

	return lector.data


# ═════════════════════════════════════════════════════════════════════════════
#  ACCESO CÓMODO A LOS DATOS  (getters)
# ═════════════════════════════════════════════════════════════════════════════

func get_modelo_tren(id: String) -> Dictionary:
	return modelos_trenes.get(id, {})

func get_linea(id: String) -> Dictionary:
	return lineas.get(id, {})

func get_estacion(id: String) -> Dictionary:
	return estaciones_por_id.get(id, {})

func get_incidencia(id: String) -> Dictionary:
	return incidencias.get(id, {})

## Devuelve el Color de Godot de una línea a partir de su código hexadecimal.
func get_color_linea(id: String) -> Color:
	var datos := get_linea(id)
	if datos.has("color"):
		return Color.html(str(datos["color"]))
	return Color.WHITE

## Devuelve la topología de una línea (estaciones con su espera y tramos entre
## ellas), tal y como la calculó generar_horario_oficial.py a partir de
## los horarios oficiales. Es la secuencia REAL de paradas (no una heurística
## geográfica) y la usan tanto MapaCatalunya (para construir la vía) como
## ScheduleManager/Tren (para el ritmo de viaje y parada).
func get_topologia_linea(id_linea: String) -> Dictionary:
	return horario_topologia.get(id_linea, {})

## Ids de estación en el orden real de la línea.
func get_orden_estaciones_linea(id_linea: String) -> Array:
	var estaciones_topo := get_topologia_linea(id_linea).get("estaciones", []) as Array
	var res: Array = []
	for e in estaciones_topo:
		res.append(str((e as Dictionary).get("id", "")))
	return res

## tramos[i] = segundos de viaje (constantes) entre la estación i y la i+1.
func get_tramos_linea(id_linea: String) -> Array:
	return get_topologia_linea(id_linea).get("tramos", []) as Array

## esperas[i] = segundos de parada comercial normalizada en la estación i.
func get_esperas_linea(id_linea: String) -> Array:
	var estaciones_topo := get_topologia_linea(id_linea).get("estaciones", []) as Array
	var res: Array = []
	for e in estaciones_topo:
		res.append(float((e as Dictionary).get("espera_seg", 20.0)))
	return res

## Salidas programadas de una línea: [{"origen":.., "destino":.., "salida":"HH:MM:SS"}, ...].
## Vacío si la línea aún no tiene servicios asignados (p. ej. R3, en obras).
func get_servicios_linea(id_linea: String) -> Array:
	return horario_servicios.get(id_linea, []) as Array


# ═════════════════════════════════════════════════════════════════════════════
#  RELOJ DEL JUEGO
# ═════════════════════════════════════════════════════════════════════════════

## Devuelve la hora actual del juego como texto "HH:MM".
func hora_actual() -> String:
	var total_min := int(tiempo_juego_seg / 60.0)
	# Dividimos entre 60.0 (float) a propósito: si dividiéramos entre 60 (entero)
	# Godot lanzaría el aviso "Integer division", que con tu configuración
	# (Treat Warnings as Errors) se convierte en error de compilación.
	var h := int(total_min / 60.0) % 24
	var m := total_min % 60
	return "%02d:%02d" % [h, m]

## Reinicia el reloj al inicio de la jornada (útil para el banco de pruebas).
func reiniciar_jornada() -> void:
	tiempo_juego_seg = HORA_INICIO_SEG
	jornada_terminada = false
	_ultimo_minuto_emitido = -1
	tiempo_actualizado.emit(hora_actual(), tiempo_juego_seg)

## Salta a una hora concreta (formato 24 h). Ej: ir_a_hora(8, 30)  ->  08:30
func ir_a_hora(horas: int, minutos: int = 0) -> void:
	tiempo_juego_seg = clampf(horas * 3600 + minutos * 60, HORA_INICIO_SEG, HORA_FIN_SEG)
	jornada_terminada = false
	_ultimo_minuto_emitido = -1
	tiempo_actualizado.emit(hora_actual(), tiempo_juego_seg)


# ═════════════════════════════════════════════════════════════════════════════
#  CONTROL DE VELOCIDAD
# ═════════════════════════════════════════════════════════════════════════════

## Fija la velocidad usando una etiqueta del diccionario VELOCIDADES.
## Ej: establecer_velocidad("X5")
func establecer_velocidad(etiqueta: String) -> void:
	if not VELOCIDADES.has(etiqueta):
		push_warning("[Global] Velocidad desconocida: " + etiqueta)
		return
	multiplicador_velocidad = VELOCIDADES[etiqueta]
	velocidad_cambiada.emit(multiplicador_velocidad)

func pausar() -> void:
	if multiplicador_velocidad > 0.0:
		_velocidad_previa = multiplicador_velocidad
	multiplicador_velocidad = 0.0
	velocidad_cambiada.emit(multiplicador_velocidad)

func reanudar() -> void:
	multiplicador_velocidad = _velocidad_previa
	velocidad_cambiada.emit(multiplicador_velocidad)

func esta_en_pausa() -> bool:
	return multiplicador_velocidad <= 0.0


# ═════════════════════════════════════════════════════════════════════════════
#  KPI OBJETIVO  ->  MINIMIZAR (minutos de retraso × pasajeros afectados)
# ═════════════════════════════════════════════════════════════════════════════

## Registra un evento de retraso. Llámalo desde la lógica de incidencias/trenes.
##   minutos   = retraso provocado por el evento
##   pasajeros = nº de viajeros afectados por ese retraso
func registrar_retraso(minutos: float, pasajeros: int) -> void:
	total_minutos_retraso += minutos
	total_pasajeros_afectados += pasajeros
	kpi_pasajeros_minuto += minutos * float(pasajeros)
	kpi_actualizado.emit(kpi_pasajeros_minuto)

## Pone el KPI a cero (al empezar una partida o una prueba nueva).
func reiniciar_kpi() -> void:
	total_minutos_retraso = 0.0
	total_pasajeros_afectados = 0
	kpi_pasajeros_minuto = 0.0
	kpi_actualizado.emit(kpi_pasajeros_minuto)


# ═════════════════════════════════════════════════════════════════════════════
#  PROYECCIÓN GEOGRÁFICA  (lat/lon  ->  píxeles 2D)
# ═════════════════════════════════════════════════════════════════════════════
#  Usamos una proyección "equirectangular" sencilla pero corregida por la
#  latitud (un grado de longitud mide menos cuanto más al norte estás). Para una
#  región pequeña como Catalunya es más que suficiente.
#
#  El proceso tiene dos pasos:
#    1) preparar_proyeccion(): calcula UNA vez la escala y el desplazamiento,
#       de modo que TODAS las estaciones quepan en el área de dibujo sin
#       deformar la proporción.
#    2) proyectar(): convierte cualquier (lat, lon) en un Vector2 de píxeles.
# ─────────────────────────────────────────────────────────────────────────────

func preparar_proyeccion(area: Vector2, margen: float) -> void:
	if estaciones.is_empty():
		return

	# 1) Caja geográfica que contiene todas las estaciones.
	var lat_min: float = INF
	var lat_max: float = -INF
	var lon_min: float = INF
	var lon_max: float = -INF
	for est in estaciones:
		lat_min = minf(lat_min, float(est["lat"]))
		lat_max = maxf(lat_max, float(est["lat"]))
		lon_min = minf(lon_min, float(est["lon"]))
		lon_max = maxf(lon_max, float(est["lon"]))

	# 2) Corrección por latitud: 1° de longitud = cos(lat) · (1° de latitud).
	var lat_centro_rad := deg_to_rad((lat_min + lat_max) / 2.0)
	var cos_lat := cos(lat_centro_rad)

	# 3) Tamaño del mapa en "grados corregidos".
	var ancho_geo := (lon_max - lon_min) * cos_lat
	var alto_geo := (lat_max - lat_min)
	if ancho_geo <= 0.0: ancho_geo = 0.0001
	if alto_geo <= 0.0: alto_geo = 0.0001

	# 4) Escala ÚNICA (la menor) para que todo quepa sin deformar la proporción.
	var area_util := area - Vector2(margen * 2.0, margen * 2.0)
	var escala := minf(area_util.x / ancho_geo, area_util.y / alto_geo)

	# 5) Centramos el mapa dentro del área útil.
	var ancho_px := ancho_geo * escala
	var alto_px := alto_geo * escala
	var offset := Vector2(
		margen + (area_util.x - ancho_px) / 2.0,
		margen + (area_util.y - alto_px) / 2.0
	)

	# Guardamos lo necesario para `proyectar()`.
	_proy_lon_min = lon_min
	_proy_lat_max = lat_max
	_proy_cos_lat = cos_lat
	_proy_escala = escala
	_proy_offset = offset


## Convierte coordenadas geográficas (lat, lon) en píxeles de pantalla.
## OJO: en pantalla la Y crece hacia ABAJO, por eso restamos la latitud desde el
## máximo (así el norte queda arriba).
func proyectar(lat: float, lon: float) -> Vector2:
	var x := (lon - _proy_lon_min) * _proy_cos_lat * _proy_escala
	var y := (_proy_lat_max - lat) * _proy_escala
	return Vector2(x, y) + _proy_offset

## Atajo: proyecta directamente una estación por su id.
func proyectar_estacion(id: String) -> Vector2:
	var est := get_estacion(id)
	if est.is_empty():
		return Vector2.ZERO
	return proyectar(est["lat"], est["lon"])

## Devuelve cuántos píxeles del mapa equivalen a 1 km, según la proyección
## actual. Utilidad general de la proyección (los trenes ya no la usan para
## su ritmo: van por los tramos constantes de horario_topologia.json).
## (1 grado de latitud ≈ 111 km; _proy_escala son píxeles por "grado corregido".)
func pixeles_por_km() -> float:
	return _proy_escala / 111.0

## Distancia geográfica REAL (aproximada) en km entre dos puntos lat/lon,
## con la misma corrección por latitud que usa preparar_proyeccion() (1° de
## longitud = cos(lat) · 1° de latitud): de sobra de precisión para repartir
## cantones proporcionalmente a la distancia — no hace falta la fórmula
## completa de Haversine para una red del tamaño de Catalunya.
func distancia_km(lat1: float, lon1: float, lat2: float, lon2: float) -> float:
	var cos_lat := cos(deg_to_rad((lat1 + lat2) / 2.0))
	var dlat_km := (lat2 - lat1) * 111.0
	var dlon_km := (lon2 - lon1) * cos_lat * 111.0
	return sqrt(dlat_km * dlat_km + dlon_km * dlon_km)
