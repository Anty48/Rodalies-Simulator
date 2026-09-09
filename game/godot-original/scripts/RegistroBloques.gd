class_name RegistroBloques
extends RefCounted
## ════════════════════════════════════════════════════════════════════════════
##  RegistroBloques.gd  —  Almacén central de bloques compartido por todo el mapa
## ════════════════════════════════════════════════════════════════════════════
##  Hay UN solo registro para todo el mapa. Cuando alguien (un tren o un semáforo)
##  pide el bloque del tramo "DESDE->HASTA", se lo damos: si no existía, lo
##  creamos; si ya existía (porque otra línea pasa por ahí), devolvemos EL MISMO.
##  Esa es la magia de "compartir la vía de verdad": misma clave, mismo objeto.
## ════════════════════════════════════════════════════════════════════════════

var _bloques: Dictionary = {}   # "DESDE>HASTA" -> Bloque


func bloque(desde: String, hasta: String) -> Bloque:
	var clave := desde + ">" + hasta
	if not _bloques.has(clave):
		_bloques[clave] = Bloque.new()
	var b: Bloque = _bloques[clave]
	return b
