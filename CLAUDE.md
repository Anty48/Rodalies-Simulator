# CLAUDE.md — Simulador Rodalies de Catalunya (Godot 4 / GDScript)

> Fichero de contexto para Claude Code. Léelo entero antes de tocar nada.
> **Idioma:** todo el código y los comentarios van en **español**, con comentarios
> didácticos abundantes (la autora viene de Python y está aprendiendo Godot).

---

## 1. Qué es este proyecto

Simulador 2D **100 % data-driven** de la red de cercanías **Rodalies de Catalunya**.
Se cargan estaciones, líneas, modelos de tren e incidencias desde JSON y se
construye el mapa, los trenes en movimiento, la señalización por cantones y el
enclavamiento de estaciones **por código** (casi nada se monta a mano en escenas).

Objetivo del simulador: ver circular los trenes por vía doble sobre un mapa
geográfico real, gestionar bloqueos/semáforos y (fase pendiente) incidencias con
un KPI de retraso acumulado.

---

## 2. Restricciones del entorno — CRÍTICO, se incumple y no compila

El proyecto tiene **"Treat Warnings as Errors" ACTIVADO** y tipado estricto. Reglas
que hay que respetar SIEMPRE:

- **Nada de inferir tipo desde un `Variant`.** `var x := diccionario[clave]` da el
  error *"Cannot infer the type of 'x'"*. Soluciones:
  - Envolver con conversión explícita: `var x := int(d[k])`, `float(...)`, `str(...)`, `bool(...)`.
  - O castear: `var x := d.get(k, null) as MiClase`.
  - O declarar el tipo: `var x: Variant = d[k]`.
- **Nada de división entera con `/` entre dos int** (da warning→error). Usa el truco
  del float: `algo / 60.0`, o `int(a / float(b))`. El módulo `%` sí es seguro.
- **Sin variables/parámetros sin usar.** Prefija los parámetros no usados con `_`
  (p. ej. `func _on_input_event(_vp, event, _shape_idx)`).
- Usa las funciones tipadas: `minf/maxf/mini/maxi`, `clampf`, `absf`, `Array[Tipo]`,
  `PackedVector2Array`, etc.
- **Al añadir un script con `class_name` nuevo**, Godot no lo reconoce hasta hacer
  **Proyecto → Recargar proyecto actual**. Si aparece *"Identifier not declared"*
  en una clase que acabas de crear, es esto (no un error de código).
- Con un **MCP de Godot conectado** (ver §9) puedes *validar sintaxis y leer errores
  del editor tú mismo* — úsalo siempre antes de dar por buena una tanda de cambios.
  Sin MCP, razona el GDScript 4 con cuidado porque no hay forma de ejecutarlo.

---

## 3. Preferencias de la autora que NO se deben cambiar sin permiso

- `Global.FACTOR_BASE_TIEMPO := 30.0` — ritmo del reloj de juego. **No tocar.**
- `OFFSET_VIA := 0.5` — separación de cada vía respecto al centro. Aparece en
  **`Tren.gd` y `MapaCatalunya.gd` y DEBE COINCIDIR** en ambos. A ella le gusta 0.5.
- Formato: tabs para indentar (no espacios), comentarios en español.

---

## 4. Estructura del proyecto

```
res://
├── datos/
│   ├── estaciones.json        # 73 estaciones: id, nombre, lat, lon, vias, lineas[]
│   ├── lineas.json            # R1..R8 con color hex y estaciones
│   ├── modelos_trenes.json    # 447, 465, 450, 490 (velocidad_max, etc.)
│   └── incidencias.json       # averia_puertas, caida_catenaria, averia_mecanica
├── escenas/
│   ├── MenuPrincipal.tscn     # menú inicial (selector de mapa)
│   └── MapaCatalunya.tscn     # escena del simulador (nodo raíz con MapaCatalunya.gd)
└── scripts/
    ├── Global.gd              # AUTOLOAD singleton "Global" (datos, reloj, proyección, KPI)
    ├── MenuPrincipal.gd       # Control del menú
    ├── MapaCatalunya.gd       # construye TODO el mapa por código (orquestador)
    ├── CamaraMapa.gd          # Camera2D: pan (arrastrar botón izq.) y zoom (rueda)
    ├── Estacion.gd            # Area2D: punto + hover (nombre/vías) + CLIC → abre panel
    ├── EstacionInterlock.gd   # (RefCounted) lógica de enclavamiento por estación
    ├── EstacionUI.gd          # (CanvasLayer) panel interactivo de estación
    ├── Tren.gd                # tren que circula por su vía, para, respeta bloqueos
    ├── Bloque.gd              # (RefCounted) un cantón entre dos estaciones (un sentido)
    ├── RegistroBloques.gd     # (RefCounted) registro global de cantones compartidos
    ├── Semaforo.gd            # Area2D: triángulo por sentido + menú de modo
    └── HUD.gd                 # CanvasLayer: botón Menú, reloj, botones de velocidad
```

**`GestorCantones.gd` está OBSOLETO** (sustituido por `Bloque.gd` + `RegistroBloques.gd`).
Se puede borrar.

---

## 5. Arquitectura y decisiones de diseño

### Global.gd (autoload "Global")
Punto central. API principal:
- Carga de datos: `cargar_todos_los_datos()`, `get_estacion(id)`, `get_linea(id)`,
  `get_modelo_tren(id)`, `get_incidencia(id)`, `get_color_linea(id)`,
  `get_estaciones_de_linea(id_linea)`.
- Reloj de jornada 05:00 → 00:00: `hora_actual()`, `ir_a_hora(h, m)`, `reiniciar_jornada()`.
- Velocidad de simulación: `establecer_velocidad(etiqueta)` (Pausa/x1/x2/x5),
  `pausar()`, `reanudar()`, `esta_en_pausa()`, var `multiplicador_velocidad`,
  const `FACTOR_BASE_TIEMPO = 30.0`.
- KPI (minutos-pasajero de retraso): `registrar_retraso(min, pasajeros)`, `reiniciar_kpi()`.
- **Proyección geográfica** equirectangular lat/lon → píxel: `preparar_proyeccion(area, margen)`,
  `proyectar(lat, lon)`, `proyectar_estacion(id)`, `pixeles_por_km()`.
- Señales: `tiempo_actualizado`, `velocidad_cambiada`, `kpi_actualizado`,
  `jornada_finalizada`, `datos_cargados`.

### Orden de rutas (limitación conocida)
`estaciones.json` **no trae la secuencia real** de paradas de cada línea. En
`MapaCatalunya._ordenar_por_cercania()` se ordena por vecino más cercano (heurística).
Es aproximado; si algún día hay datos de secuencia real, sustituir esa función.

### Vía doble + LOD por zoom
- `MapaCatalunya` dibuja, por línea: una **línea central** (`capa_centros`) y **dos
  rieles continuos** desplazados en inglete (`capa_rieles`, vía `_polilinea_desplazada`).
- **LOD real por umbral** en `_process`: si `camara.zoom < UMBRAL_VIA_DOBLE` (3.5) se
  ve solo la línea central; si no, se ven las dos vías y los semáforos. No es un
  efecto de escala, es un `if`.
- Los trenes y estaciones tienen **tamaño de pantalla constante** (`scale = 1/zoom`).

### Trenes (`Tren.gd`)
- No usan `PathFollow2D`: se posicionan a mano sobre la `Curve2D` de su línea y se
  desplazan `OFFSET_VIA` perpendicular "a la derecha" del sentido → circulan por la
  vía correcta en cada sentido.
- Máquina de estados `MARCHA / PARADO`. Paran `SEGUNDOS_PARADA` (30 s de juego).

### Cantones / semáforos de tramo (`Bloque` + `RegistroBloques` + `Semaforo`)
- Un `Bloque` = un cantón dirigido entre dos estaciones, con clave `"A>B"`.
  `RegistroBloques` es único y **compartido**: si dos líneas comparten un tramo,
  comparten el bloque. Modos: `AUTO / MANUAL_VERDE / MANUAL_ROJO`.
- `Semaforo` (Area2D) por arista dirigida, con menú al hacer clic.

### Enclavamiento de estación (`EstacionInterlock` + `EstacionUI`) — recién construido
- `EstacionInterlock` (RefCounted, uno por estación): nº de vías, **vía principal por
  dirección** ("N"/"S", solo una por dirección, reasignar deselecciona la anterior),
  **semáforo interno por vía y dirección** (solo verde/rojo, verde por defecto),
  ocupación (un tren por vía), y nombres de terminales N/S.
- Dirección **N/S por latitud**: de los dos extremos de cada línea, el de mayor
  latitud es "N" (clasifica bien Martorell como Sur en R8, etc.).
- `EstacionUI` (CanvasLayer, una sola instancia compartida): clic en estación abre el
  panel con nombre, próximo tren (ETA aproximada), terminales de cada lado, y una fila
  por vía `[S] — Vía k — [N]`. Construida con **botones estándar** (no dibujo a mano)
  para fiabilidad de clic; azul = principal, ▲Norte/▼Sur/◆N+S, verde/rojo = interno.
- **Regla de salida del tren**: sale solo si (a) su semáforo interno del sentido de
  salida está verde, (b) el cantón siguiente está libre y (c) la vía principal de la
  estación destino está libre (un tren por vía). Interno en rojo → se queda indefinido.

**Simplificaciones actuales del enclavamiento (documentadas, mejorables):**
1. ETA del próximo tren = distancia/velocidad del tren entrante más cercano cuya
   próxima parada es esa estación (ignora paradas y señales intermedias).
2. El acoplamiento "semáforo del tramo anterior en rojo" se consigue reteniendo el
   cantón de entrada (misma línea) + gating por vía-destino-ocupada (otras líneas);
   no se fuerza el rojo visual de TODOS los tramos entrantes de otras líneas.
3. Etiquetas de dirección = conjunto de terminales N/S de las líneas de esa estación
   (pueden quedar largas; se truncan en la UI).

---

## 6. Estado actual y trabajo pendiente

**Hecho:** carga de datos, menú, mapa geográfico, cámara, vía doble + LOD, trenes en
movimiento con reloj, paradas, cantones + semáforos de tramo, y enclavamiento de
estación con interfaz interactiva.

**Siguiente fase (Fase 6) — Incidencias + KPI:**
- Datos ya listos en `incidencias.json` y métricas de KPI ya en `Global.gd`.
- Idea: disparar incidencias probabilísticas (avería puertas / caída catenaria /
  avería mecánica) que retrasen trenes, alimentar `Global.registrar_retraso(min, pasajeros)`
  y mostrar el KPI (minutos-pasajero de retraso) en el HUD.

**Mejoras candidatas:** ETA real que descuente paradas/señales; secuencia real de
líneas si aparecen datos; pulido visual del panel de enclavamiento.

---

## 7. Convenciones de código (ejemplos)

```gdscript
# BIEN: tipo explícito o conversión, nunca inferir desde Variant
var vias: int = int(datos.get("vias", 1))
var inter := _interlocks.get(sid, null) as EstacionInterlock

# BIEN: evitar división entera
var minutos := int(segundos / 60.0)

# BIEN: parámetro no usado con guion bajo
func _on_mouse_entered() -> void:
    _hover = true
    queue_redraw()

# MAL (no compila con warnings-as-errors):
# var x := datos["vias"]      # no puede inferir tipo desde Variant
# var m := segundos / 60      # división entera
```

---

## 8. Cómo ejecutar / probar

1. Abrir el proyecto en Godot 4.x.
2. Escena principal: `escenas/MenuPrincipal.tscn` (o directamente `MapaCatalunya.tscn`
   para saltar al mapa).
3. Autoload "Global" debe estar en Proyecto → Ajustes → Autoload (nombre `Global`,
   ruta `res://scripts/Global.gd`). Si falla la carga de datos, revisar este autoload.
4. Tras crear/renombrar clases con `class_name`: **Recargar proyecto actual**.
5. Prueba del enclavamiento: hacer zoom hasta ver dos vías, clic en una estación
   (p. ej. Sants con 14 vías), asignar vías principales, poner un interno en rojo y
   comprobar que el tren se queda parado y el de detrás espera.

---

## 9. Herramientas recomendadas para el setup (ver también el chat)

- **MCP de Godot** (lo más valioso): permite ejecutar el proyecto, leer errores del
  editor, validar sintaxis GDScript y hacer capturas. Cierra el bucle "editar → probar
  → corregir". Opciones open-source como `mkdevkit/godot-mcp` o `Coding-Solo/godot-mcp`.
- **gdtoolkit** (`gdlint` + `gdformat`): linter y formateador de GDScript para pasar
  antes de commit y cazar problemas de estilo/tipado.
- **godot-tools** (extensión LSP) si se edita desde VS Code/Cursor.
- **GUT** (Godot Unit Test) si se quieren tests de la lógica pura (`EstacionInterlock`,
  `Bloque`, proyección de `Global`).
