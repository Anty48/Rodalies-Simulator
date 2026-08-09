class_name EstacionInterlock
extends RefCounted
## ════════════════════════════════════════════════════════════════════════════
##  EstacionInterlock.gd  —  El "enclavamiento" de una estación
## ════════════════════════════════════════════════════════════════════════════
##  Para cada estación guarda:
##   - num_vias andenes.
##   - CORREDORES: partición de esas vías en grupos INDEPENDIENTES (ver más
##     abajo). Por defecto hay un único corredor que cubre todas las vías y
##     todas las líneas (el comportamiento de siempre); una estación puede
##     declarar en estaciones.json varios corredores (p. ej. Barcelona El Clot:
##     2 vías para R1, otras 2 para R2/R2_NORD) para que el tráfico de uno no
##     bloquee al del otro, tal y como pasa en la infraestructura real.
##   - La VÍA PRINCIPAL de cada LADO ("A"/"B": los dos extremos de la línea,
##     NO puntos cardinales — una línea no tiene "norte" ni "sur", tiene dos
##     terminales), UNA POR CORREDOR. Solo una vía por lado y corredor (puede
##     ser la misma vía para los dos lados).
##   - Un SEMÁFORO INTERNO por vía y lado: solo verde/rojo (sin automático),
##     verde por defecto. Decide si un tren ESTACIONADO puede salir hacia ese lado.
##   - Qué tren ocupa cada vía (máximo uno).
##   - Los nombres de terminal de cada lado (para etiquetar la interfaz con el
##     nombre real del destino, p. ej. "Manresa", en vez de una etiqueta N/S).
## ════════════════════════════════════════════════════════════════════════════

var id: String = ""
var nombre: String = ""
var num_vias: int = 1
var terminales_a: String = ""
var terminales_b: String = ""

# --- Corredores ---
# _vias_por_corredor[c] = Array[int] de índices de vía que pertenecen al
# corredor c (partición de 0..num_vias-1: cada vía cae en UN solo corredor).
# _corredor_de_linea[id_linea] = índice de corredor (0 si la línea no aparece
# en ningún grupo declarado, o si la estación no define corredores).
# _corredor_de_via[via] = índice de corredor al que pertenece esa vía.
var _vias_por_corredor: Array = []
var _corredor_de_linea: Dictionary = {}
var _corredor_de_via: Dictionary = {}
var _lineas_por_corredor: Array = []   # por corredor: texto "R1 / R3 / R4" (solo para la UI; vacío si no hay corredores)

var _verde: Array = []        # por vía: { "A": bool, "B": bool }  (true = verde)
var _principal: Array = []    # por corredor: { "A": int, "B": int }  (índices de vía)
var _ocupada: Array = []      # por vía: Tren o null
var _reservada: Array = []    # por vía: Tren o null (vía única — ver reservar())
var _term_a: Dictionary = {}  # conjunto de nombres de terminal del lado A
var _term_b: Dictionary = {}

# --- Pasajeros esperando (modelo por lotes, SIN nodo/objeto por pasajero) ---
# id_estacion_destino -> cantidad de pasajeros acumulados que quieren ir ahí.
# Lo rellena el generador periódico (por coeficiente de afluencia) y lo vacía
# un Tren al llegar y absorber los lotes que van en su sentido de circulación.
var pasajeros_por_destino: Dictionary = {}

func anadir_pasajeros(id_destino: String, cantidad: int) -> void:
	if cantidad <= 0:
		return
	pasajeros_por_destino[id_destino] = int(pasajeros_por_destino.get(id_destino, 0)) + cantidad

func pasajeros_para(id_destino: String) -> int:
	return int(pasajeros_por_destino.get(id_destino, 0))

## Retira hasta `cantidad` pasajeros con destino `id_destino` (puede haber
## menos esperando; devuelve cuántos se retiraron de verdad). Lo usa un Tren
## al recogerlos -- ver Tren._recoger_pasajeros().
func extraer_pasajeros(id_destino: String, cantidad: int) -> int:
	var disponibles := pasajeros_para(id_destino)
	var retirados := mini(disponibles, maxi(cantidad, 0))
	if retirados <= 0:
		return 0
	var restante := disponibles - retirados
	if restante <= 0:
		pasajeros_por_destino.erase(id_destino)
	else:
		pasajeros_por_destino[id_destino] = restante
	return retirados

func pasajeros_esperando_total() -> int:
	var total := 0
	for cantidad in pasajeros_por_destino.values():
		total += int(cantidad)
	return total


## `p_corredores` (opcional) viene de estaciones.json: Array de
## { "lineas": ["R1", ...] } — una entrada por corredor, en el orden en que
## deben repartirse las vías. Si se omite, toda la estación es un único
## corredor (comportamiento de siempre).
func _init(p_id: String, p_nombre: String, p_num_vias: int, p_corredores: Array = []) -> void:
	id = p_id
	nombre = p_nombre
	num_vias = maxi(p_num_vias, 1)
	for i in num_vias:
		_verde.append({"A": true, "B": true})
		_ocupada.append(null)
		_reservada.append(null)
	_configurar_corredores(p_corredores)


## Reparte las vías (por igual, en bloques consecutivos) entre los corredores
## declarados y registra qué líneas pertenecen a cada uno. La vía principal
## por defecto de cada corredor son sus dos primeras vías (una por lado, o la
## misma si el corredor solo tiene una).
func _configurar_corredores(p_corredores: Array) -> void:
	if p_corredores.is_empty():
		var todas: Array = []
		for v in num_vias:
			todas.append(v)
			_corredor_de_via[v] = 0
		_vias_por_corredor.append(todas)
		_principal.append({"A": 0, "B": 1 if num_vias > 1 else 0})
		return

	var n := p_corredores.size()
	var base := int(num_vias / float(n))
	var resto := num_vias % n
	var cursor := 0
	for i in n:
		var cuantas := base + (1 if i < resto else 0)
		var vias_grupo: Array = []
		for _k in cuantas:
			vias_grupo.append(cursor)
			_corredor_de_via[cursor] = i
			cursor += 1
		_vias_por_corredor.append(vias_grupo)

		var grupo := p_corredores[i] as Dictionary
		var lineas_grupo := grupo.get("lineas", []) as Array
		var nombres_linea := PackedStringArray()
		for linea in lineas_grupo:
			_corredor_de_linea[str(linea)] = i
			nombres_linea.append(str(linea))
		_lineas_por_corredor.append(" / ".join(nombres_linea))

		var v0 := int(vias_grupo[0]) if vias_grupo.size() > 0 else 0
		var v1 := int(vias_grupo[1]) if vias_grupo.size() > 1 else v0
		_principal.append({"A": v0, "B": v1})


func corredor_de_linea(id_linea: String) -> int:
	return int(_corredor_de_linea.get(id_linea, 0))

## Para la UI (EstacionUI): cuántos corredores independientes tiene esta
## estación (1 si no declara "corredores" en estaciones.json), qué vías
## pertenecen a cada uno y qué líneas lo usan (para etiquetarlo).
func num_corredores() -> int:
	return _vias_por_corredor.size()

func vias_de_corredor(c: int) -> Array:
	if c < 0 or c >= _vias_por_corredor.size():
		return []
	return _vias_por_corredor[c]

func etiqueta_corredor(c: int) -> String:
	if c < 0 or c >= _lineas_por_corredor.size():
		return ""
	return str(_lineas_por_corredor[c])


# --- Vías principales ---
## Vía principal hacia `lado` para el CORREDOR de `id_linea` (cada corredor
## tiene la suya propia, así que la misma estación puede tener varias vías
## "principal lado A" a la vez, una por corredor).
func via_principal(lado: String, id_linea: String) -> int:
	return via_principal_de_corredor(lado, corredor_de_linea(id_linea))

func via_principal_de_corredor(lado: String, corredor: int) -> int:
	var c := corredor if corredor >= 0 and corredor < _principal.size() else 0
	return int(_principal[c].get(lado, 0))

## Asignar no necesita `id_linea`: la vía ya pertenece a un corredor concreto,
## así que solo se reasigna la principal de ESE corredor (los demás no se tocan).
func asignar_principal(via: int, lado: String) -> void:
	if via < 0 or via >= num_vias:
		return
	var c := int(_corredor_de_via.get(via, 0))
	_principal[c][lado] = via   # la anterior en ese lado y corredor queda deseleccionada

func es_principal(via: int, lado: String) -> bool:
	var c := int(_corredor_de_via.get(via, 0))
	return int(_principal[c].get(lado, -1)) == via


# --- Semáforos internos ---
func semaforo_verde(via: int, lado: String) -> bool:
	if via < 0 or via >= num_vias:
		return true
	return bool(_verde[via].get(lado, true))

func alternar_semaforo(via: int, lado: String) -> void:
	if via >= 0 and via < num_vias:
		_verde[via][lado] = not bool(_verde[via][lado])


# --- Ocupación (un tren por vía) ---
func ocupar(via: int, tren) -> void:
	if via >= 0 and via < num_vias:
		_ocupada[via] = tren
		_reservada[via] = null   # la ocupación real sustituye cualquier reserva previa (normalmente la suya propia)
		# Un tren solo puede tener UNA reserva de vía única viva a la vez (la
		# del hop que acaba de completar), pero puede acabar ocupando una vía
		# DISTINTA a la reservada: si su servicio termina aquí mismo y la
		# estación no tiene apartadero real (0 vías no principales, típico de
		# un cruce de vía única), _ocupar_via_secundaria_obligatoria() cae al
		# fallback "cualquier vía libre" en vez de la reservada. Sin esto, esa
		# reserva original se queda huérfana para siempre y bloquea a todo el
		# tráfico detrás (visto en Granollers-Canovelles con los horarios
		# reales: un giro corto ahí colapsaba toda la R3 vía única).
		if tren != null:
			for v in range(num_vias):
				if v != via and _reservada[v] == tren:
					_reservada[v] = null

func liberar(via: int) -> void:
	if via >= 0 and via < num_vias:
		_ocupada[via] = null

func tren_en(via: int):
	if via < 0 or via >= num_vias:
		return null
	return _ocupada[via]


# --- Reserva anticipada (solo vía única — ver checklist R3 y Tren._intentar_partir) ---
# Un tren que se compromete a un tramo de VÍA ÚNICA tarda minutos en llegar de
# verdad: si solo mirásemos la ocupación INSTANTÁNEA al partir, dos trenes
# podrían "ver" la misma vía libre en momentos distintos y converger los dos
# hacia ella — el segundo se queda esperando fuera indefinidamente y, al
# tener ya el testigo de vía única de su sentido, bloquea también al tráfico
# contrario (deadlock real, visto en Parets del Vallès). La reserva cierra
# ese hueco: se marca la vía como "prometida" a un tren concreto en el
# instante en que decide partir hacia ella (antes de llegar), así ningún
# otro puede comprometerse también a esa misma vía mientras tanto.
func reservar(via: int, tren) -> void:
	if via >= 0 and via < num_vias:
		_reservada[via] = tren

func liberar_reserva(via: int) -> void:
	if via >= 0 and via < num_vias:
		_reservada[via] = null

## ¿Puede `tren` comprometerse ahora mismo a `via`? Ni ocupada físicamente ni
## prometida a OTRO tren que también viene de camino (si la reserva ya es
## suya, cuenta como libre — comprobación repetida en frames sucesivos antes
## de llegar).
func via_libre_para_reserva(via: int, tren) -> bool:
	if via < 0 or via >= num_vias:
		return false
	if _ocupada[via] != null:
		return false
	return _reservada[via] == null or _reservada[via] == tren

## Como primera_via_libre(), pero exigiendo también via_libre_para_reserva():
## la primera vía del corredor de `id_linea` que `tren` puede reservar ahora
## mismo. Devuelve -1 si ninguna vía del corredor cumple ambas condiciones.
func primera_via_libre_reservable(id_linea: String, tren) -> int:
	var c := corredor_de_linea(id_linea)
	for v in (_vias_por_corredor[c] as Array):
		var vi := int(v)
		if via_libre_para_reserva(vi, tren):
			return vi
	return -1


## Cuántas vías del corredor de `id_linea` NO son la principal de ningún lado
## ahora mismo (apartaderos reales de ESE corredor). Con 1 o 2 vías en el
## corredor, A y B ya las cubren todas (o la misma) y esto da 0: ese corredor
## no sirve para aparcar un tren que solo espera su horario (lo bloquearía
## todo), solo para tráfico de paso.
func num_vias_no_principales(id_linea: String) -> int:
	var c := corredor_de_linea(id_linea)
	var principales := {}
	principales[int(_principal[c]["A"])] = true
	principales[int(_principal[c]["B"])] = true
	return (_vias_por_corredor[c] as Array).size() - principales.size()


## Una vía libre del corredor de `id_linea` que NO sea la principal de ningún
## lado (un apartadero). La usan los trenes que llegan a su terminal (o a una
## terminal intermedia de giro corto) y van a quedarse aparcados un buen rato
## esperando su próxima asignación: así dejan libre la vía principal para que
## los trenes que vengan detrás puedan entrar sin más. Devuelve -1 si no hay
## ninguna (p. ej. un corredor de solo 2 vías).
func via_libre_no_principal(id_linea: String) -> int:
	var c := corredor_de_linea(id_linea)
	for v in (_vias_por_corredor[c] as Array):
		var vi := int(v)
		if _ocupada[vi] == null and not es_principal(vi, "A") and not es_principal(vi, "B"):
			return vi
	return -1

## Cualquier vía libre del corredor de `id_linea`, sin distinguir si es
## principal (último recurso cuando no queda ningún apartadero disponible EN
## ESE CORREDOR — nunca se recurre a vías de otro corredor, aunque estén
## libres: son físicamente independientes). Devuelve -1 si no hay ninguna.
func primera_via_libre(id_linea: String) -> int:
	return primera_via_libre_de_corredor(corredor_de_linea(id_linea))

## Igual que primera_via_libre() pero recibiendo ya el índice de corredor en
## vez de una línea (para cuando ya se ha resuelto el corredor de antemano).
func primera_via_libre_de_corredor(corredor: int) -> int:
	var c := corredor if corredor >= 0 and corredor < _vias_por_corredor.size() else 0
	for v in (_vias_por_corredor[c] as Array):
		var vi := int(v)
		if _ocupada[vi] == null:
			return vi
	return -1


# --- Terminales (para las etiquetas de la interfaz) ---
func add_terminal(lado: String, nombre_terminal: String) -> void:
	if nombre_terminal == "":
		return
	if lado == "A":
		_term_a[nombre_terminal] = true
	else:
		_term_b[nombre_terminal] = true
	terminales_a = _texto_terminales(_term_a)
	terminales_b = _texto_terminales(_term_b)

func _texto_terminales(d: Dictionary) -> String:
	var partes := PackedStringArray()
	for k in d.keys():
		partes.append(str(k))
	return " / ".join(partes)
