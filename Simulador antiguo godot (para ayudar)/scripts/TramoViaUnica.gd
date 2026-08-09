class_name TramoViaUnica
extends RefCounted
## ════════════════════════════════════════════════════════════════════════════
##  TramoViaUnica.gd  —  "Testigo" (token) de un tramo físico de VÍA ÚNICA
## ════════════════════════════════════════════════════════════════════════════
##  A diferencia de la vía doble (donde "A>B" y "B>A" son dos Bloque totalmente
##  independientes — cada sentido tiene su propio raíl), en vía única los dos
##  sentidos comparten el MISMO raíl físico: solo uno de los dos puede estar
##  "circulando" por el tramo a la vez, igual que el testigo/bastón piloto de
##  la señalización ferroviaria real de vía única.
##
##  Hay UNA instancia por tramo NO dirigido (un mismo objeto para "A>B" y
##  "B>A"), compartida entre las dos cadenas de cantones de ese tramo (ver
##  MapaCatalunya._crear_cantones_tramo). Cada Bloque de AMBAS cadenas guarda
##  una referencia a este mismo testigo más la clave dirigida ("sentido") que
##  representa (ver Bloque.configurar_via_unica) — así CUALQUIER cantón de la
##  cadena, no solo el primero o el último, puede consultar "¿está el tramo
##  libre para mi sentido?" (ver Bloque.en_rojo()/via_unica_disponible()).
##
##  Solo se reserva/libera una vez por tren y por tránsito completo del tramo
##  (al partir de la estación origen y al llegar de verdad a la destino — ver
##  Tren._intentar_partir()/_llegar_a_estacion()), nunca en cada cantón
##  intermedio: eso es lo que permite que VARIOS trenes del MISMO sentido se
##  sigan por el tramo sin pisarse el testigo unos a otros (el cantonamiento
##  intermedio ya existente, ver MapaCatalunya._crear_cantones_tramo, se
##  encarga de que no se choquen por detrás).
## ════════════════════════════════════════════════════════════════════════════

var _sentido_activo: String = ""   # "" = libre; si no, la clave dirigida que lo tiene reservado
var _trenes_activos: int = 0


## ¿Puede un tren de `sentido` entrar ahora mismo? Sí si el tramo está libre,
## o si ya lo tiene reservado ESE MISMO sentido (trenes en fila, uno detrás
## de otro, todos a favor).
func disponible_para(sentido: String) -> bool:
	return _sentido_activo == "" or _sentido_activo == sentido


## Un tren de `sentido` acaba de partir hacia este tramo. No-op si otro
## sentido ya lo tenía reservado (no debería llamarse en ese caso: quien
## llama ya ha comprobado disponible_para() antes de dejar partir al tren).
func reservar(sentido: String) -> void:
	if _sentido_activo == "":
		_sentido_activo = sentido
	if _sentido_activo == sentido:
		_trenes_activos += 1


## Un tren de `sentido` acaba de completar el tramo entero (llegó de verdad a
## la estación destino). El tramo solo vuelve a quedar libre cuando el ÚLTIMO
## tren de ese sentido lo abandona.
func liberar(sentido: String) -> void:
	if _sentido_activo != sentido:
		return
	_trenes_activos = maxi(0, _trenes_activos - 1)
	if _trenes_activos == 0:
		_sentido_activo = ""
