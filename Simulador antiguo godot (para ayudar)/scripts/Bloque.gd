class_name Bloque
extends RefCounted
## ════════════════════════════════════════════════════════════════════════════
##  Bloque.gd  —  Un tramo físico DIRIGIDO entre dos estaciones (p. ej. A->B)
## ════════════════════════════════════════════════════════════════════════════
##  Representa un cantón de vía en un sentido concreto. Como se identifica por las
##  estaciones (no por la línea), TODAS las líneas que recorran ese mismo tramo
##  en ese mismo sentido comparten este objeto: la vía se comparte de verdad.
##
##  El sentido contrario (B->A) es un Bloque DISTINTO: eso nos da la vía doble.
##
##  Modo del semáforo que lo protege:
##    - AUTO          -> rojo si hay tránsito físico O si la vía PRINCIPAL de la
##                        estación destino (para el lado que llega) está ocupada.
##                        Esto es SIEMPRE respecto a la vía principal en concreto,
##                        nunca "cualquier vía libre": ese matiz (un tren cuyo
##                        SERVICIO termina justo ahí no necesita la principal)
##                        es una condición DE CADA TREN, no del tramo — lo
##                        decide Tren._intentar_partir() por su cuenta, porque
##                        este Bloque lo comparten potencialmente varias líneas
##                        y no puede saber si el próximo en llegar termina aquí
##                        o solo pasa de largo.
##    - MANUAL_ROJO   -> siempre rojo (el jugador lo cierra a mano).
##    - MANUAL_VERDE  -> siempre verde (el jugador lo fuerza abierto).
##
##  IMPORTANTE (corrección de cantonamiento): que un tren esté APARTADO en una
##  vía SECUNDARIA de la estación destino ya NO pone este semáforo en rojo. Solo
##  importa la vía PRINCIPAL asignada a ese lado — así, apartar un tren en una
##  vía libre reasignándole otra vía principal desatasca la circulación general
##  en vez de colapsarla.
##
##  VÍA ÚNICA (ver TramoViaUnica.gd): un Bloque que pertenece a un tramo de vía
##  única lleva además una referencia al TESTIGO compartido de ese tramo (el
##  mismo objeto para los dos sentidos) y la clave dirigida que representa. Si
##  el sentido contrario tiene el testigo, este Bloque se pinta/actúa en rojo
##  igual que si estuviera físicamente ocupado — así se evita el choque
##  frontal sin tocar para nada la lógica de vía doble existente.
## ════════════════════════════════════════════════════════════════════════════

enum Modo { AUTO, MANUAL_VERDE, MANUAL_ROJO }

var ocupado: bool = false     # tránsito físico: hay un tren circulando POR este cantón abierto
var modo: Modo = Modo.AUTO

var _interlock_destino: EstacionInterlock = null
var _direccion_llegada: String = ""     # "A" o "B": lado de la estación destino que protege este bloque
var _corredor_llegada: int = 0          # corredor de la estación destino que protege este bloque

var _tramo_via_unica: TramoViaUnica = null
var _sentido_via_unica: String = ""


## Asocia este bloque con la estación a la que conduce, el lado ("A"/"B") de
## esa estación al que corresponde y el CORREDOR (ver EstacionInterlock) cuya
## vía principal hay que vigilar. El corredor se fija una sola vez, con el de
## la línea que primero creó este bloque (las líneas que comparten tramo
## exacto comparten también corredor en la práctica; ver MapaCatalunya._crear_semaforo).
func configurar_destino(interlock: EstacionInterlock, direccion: String, corredor: int = 0) -> void:
	_interlock_destino = interlock
	_direccion_llegada = direccion
	_corredor_llegada = corredor


## Asocia este bloque con el testigo compartido de su tramo de vía única (ver
## TramoViaUnica.gd) y la clave dirigida ("DESDE>HASTA") que representa. Se
## llama para TODOS los cantones de la cadena, no solo el último — así
## cualquiera de ellos puede detectar tráfico de cara (checklist #4).
func configurar_via_unica(tramo: TramoViaUnica, sentido: String) -> void:
	_tramo_via_unica = tramo
	_sentido_via_unica = sentido


## ¿Libre este bloque de tráfico de cara en su tramo de vía única? Siempre
## true si el bloque no pertenece a ningún tramo de vía única (vía doble,
## comportamiento de siempre).
func via_unica_disponible() -> bool:
	return _tramo_via_unica == null or _tramo_via_unica.disponible_para(_sentido_via_unica)


## ¿Pertenece este bloque a un tramo de vía única? Lo usa Tren._intentar_partir
## para saber si debe exigir RESERVA (no solo ocupación instantánea) de la
## vía de la estación destino antes de comprometerse a partir — ver
## EstacionInterlock.reservar()/via_libre_para_reserva().
func es_via_unica() -> bool:
	return _tramo_via_unica != null


## Reserva el testigo del tramo para nuestro sentido (no-op en vía doble). Se
## llama UNA vez, al partir de la estación origen (ver Tren._intentar_partir),
## nunca en cada cantón intermedio.
func reservar_via_unica() -> void:
	if _tramo_via_unica != null:
		_tramo_via_unica.reservar(_sentido_via_unica)


## Libera el testigo del tramo (no-op en vía doble). Se llama UNA vez, al
## llegar de verdad a la estación destino (ver Tren._llegar_a_estacion).
func liberar_via_unica() -> void:
	if _tramo_via_unica != null:
		_tramo_via_unica.liberar(_sentido_via_unica)


## ¿Debe pintarse el semáforo en rojo? También es lo que consulta
## Tren._intentar_partir() para el tránsito físico del cantón (`ocupado`); la
## parte de la vía principal ocupada es solo orientativa para un tren que siga
## de largo — un tren cuyo servicio termina en la estación destino decide por
## su cuenta si le vale con cualquier vía libre (ver Tren._intentar_partir()).
##
## OJO (corredores compartidos): este Bloque puede estar compartido por VARIAS
## líneas que, en la estación destino, pertenecen a CORREDORES DISTINTOS (p.
## ej. Montcada Bifurcació: R3 en un corredor, R4/R7 en otro, todas llegando
## por el mismo tramo doble desde Torre Baró|Vallbona). Como el Bloque es UN
## solo objeto, `_corredor_llegada` solo puede guardar el de la línea que lo
## creó PRIMERO (ver configurar_destino) — válido para el semáforo VISUAL
## (un posible falso "rojo" para las demás líneas es aceptable, ver
## en_rojo_para), pero NO para decidir si un tren puede cruzar de verdad: eso
## lo resuelve en_rojo_para() con el corredor en vivo de quien pregunta.
func en_rojo() -> bool:
	match modo:
		Modo.MANUAL_ROJO:
			return true
		Modo.MANUAL_VERDE:
			return false
		_:
			return ocupado or not via_unica_disponible() or _via_principal_ocupada(_corredor_llegada)


## Como en_rojo(), pero resolviendo la vía principal de la estación destino
## con el CORREDOR de `id_linea` EN VIVO en vez del cacheado en
## configurar_destino — necesario para que el tránsito físico real (ver
## Tren._actualizar_canton_intermedio) nunca compruebe la ocupación del
## corredor de una línea distinta a la que de verdad está cruzando. Mismo
## criterio que ya usa Tren._intentar_partir() en su condición (c) para la
## vía principal de salida: la corrección es "por tren", nunca "por Bloque".
func en_rojo_para(id_linea: String) -> bool:
	match modo:
		Modo.MANUAL_ROJO:
			return true
		Modo.MANUAL_VERDE:
			return false
		_:
			var corredor := _interlock_destino.corredor_de_linea(id_linea) if _interlock_destino != null else _corredor_llegada
			return ocupado or not via_unica_disponible() or _via_principal_ocupada(corredor)


func es_manual() -> bool:
	return modo != Modo.AUTO


func _via_principal_ocupada(corredor: int) -> bool:
	if _interlock_destino == null or _direccion_llegada == "":
		return false
	var via := _interlock_destino.via_principal_de_corredor(_direccion_llegada, corredor)
	return _interlock_destino.tren_en(via) != null
