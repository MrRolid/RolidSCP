# Ayuda de RolidSCP

RolidSCP es un gestor de archivos de doble panel para discos locales y servidores SFTP. Cada panel
tiene pestañas; una pestaña es un lugar (una carpeta local o una carpeta de un servidor) con su
propio nombre y color. Todas las funciones están en los menús, y la mayoría también en la barra de
herramientas y en la barra de teclas de función de la parte inferior. La lista de todos los atajos
de teclado está en la segunda página de esta ventana.

## Paneles y pestañas

- **Tab** cambia entre el panel izquierdo y el derecho. El panel activo tiene un encabezado más
  marcado.
- **Enter** o un doble clic abre una carpeta, un archivo comprimido (como carpeta de solo lectura) o
  un archivo con su aplicación predeterminada. Los programas y scripts nunca se inician sin
  preguntar; los archivos de un servidor nunca se ejecutan. **Backspace** sube un nivel.
- **Ctrl+T** abre un menú para una pestaña nueva: carpeta personal, favoritos, servidores guardados y
  lugares recientes; escriba para filtrarlo. **Ctrl+W** cierra la pestaña, **Ctrl+Tab** cambia de
  pestaña. Con clic derecho en una pestaña: Duplicar, Renombrar, Color, Cerrar y Cerrar las demás
  pestañas; doble clic la renombra. La información sobre una pestaña muestra su servidor, dirección y
  ruta. **Ctrl+K** busca una pestaña o un servidor guardado por su nombre.
- **Ctrl+D** añade la carpeta a favoritos o la quita. Los favoritos y las carpetas recientes están en
  el menú del botón ★ de la barra (`*` cuando la barra está oculta); **Archivo → Carpetas favoritas…**
  permite nombrarlas y ordenarlas.
- En el menú **Ctrl+T** (y el botón **+** junto a las pestañas) los favoritos se muestran siempre:
  **＋ Añadir la carpeta actual** añade la carpeta del panel activo (también en un servidor), ✏ cambia el
  nombre de un favorito y 🗑 lo quita (haga clic otra vez para confirmar, **Esc** vuelve atrás).
- Empiece a escribir para filtrar la lista; **Esc** borra el filtro.
- **Ctrl+F1**, **Ctrl+F2** y **Ctrl+Shift+F1** cambian entre la lista breve, la lista detallada y
  las miniaturas. **Alt+←** y **Alt+→** van atrás y adelante en el historial de carpetas.
- Todos los diálogos funcionan igual: **Intro** pulsa el botón resaltado, **Esc** o ✕ cancela, **Tab**
  pasa de un campo a otro. Un diálogo se mueve arrastrando su barra de título; ⓘ muestra más ayuda.
- Las pestañas, los colores y las carpetas se guardan continuamente y se restauran en el siguiente
  inicio.
- Sobre cada panel hay una barra: una lista desplegable con las carpetas de la ruta desde la raíz
  (**Alt+↓**; flechas, **Intro**, **Esc**), en un panel local también con los discos de este ordenador
  y su tamaño; después Atrás, Adelante, Subir un nivel, Carpeta raíz (**`Ctrl+\`**), Carpeta personal
  (**Ctrl+H**), Actualizar y los favoritos (★). Dentro de un archivo comprimido la lista empieza por la
  carpeta en la que está; en los resultados de una búsqueda no está disponible. **Ver → Barra sobre los
  paneles** oculta la barra; los botones `/ .. ~ *` vuelven entonces a la fila de la ruta.
- Cada parte de la ruta es un botón: un clic abre esa carpeta (también en un servidor y dentro de un
  archivo comprimido; un clic en la carpeta en la que está la actualiza), el botón central la abre
  en una pestaña nueva junto a la actual y la parte bajo el ratón se resalta. Una ruta demasiado
  larga para la fila se acorta por la izquierda con «…» (un clic en ellos abre la lista desplegable
  de la ruta); la última parte siempre se ve entera. Para escribir una ruta, haga doble clic en la
  parte vacía de la fila, pulse el lápiz al final o pulse **Ctrl+G**.
- A la derecha, la fila de la ruta muestra el espacio libre del disco (una barra fina y «Libre: X de
  Y»), su sistema de archivos (ext4, btrfs, nfs4, fuse.sshfs…) y, en los discos de este ordenador,
  una insignia con el estado del disco; en un panel estrecho se omite primero «Libre:», luego el
  tamaño del disco y luego el sistema de archivos. Dentro de un archivo comprimido muestra el disco
  en el que está. Un clic abre los detalles.
- Un clic derecho en el espacio vacío de la lista (o en `..`) ofrece **Archivo nuevo…**, **Nueva
  carpeta**, **Actualizar**, **Añadir carpeta a favoritos**, **Abrir terminal aquí** y **Propiedades
  del disco…** (espacio libre, sistema de archivos y estado del disco).
- En la lista detallada, unas líneas finas separan las columnas. Arrastre una línea entre los títulos de
  las columnas para cambiar el ancho de la columna de su derecha; un doble clic ajusta la columna a su
  contenido. Nombre ocupa el resto del panel, así que crece con una ventana más ancha; en un panel
  estrecho se ocultan Permisos y después Modificado. Los anchos se guardan por separado para el panel
  izquierdo y el derecho (también tras reiniciar); **Ver → Restablecer el ancho de las columnas** vuelve
  a los valores predeterminados. Un nombre o una extensión que no cabe termina en «…»; al pasar el ratón
  se ve completo.
- Las listas se mantienen al día solas. Una carpeta de este equipo muestra en un momento los cambios
  hechos por otros programas. Una carpeta en un servidor (o en un disco de red) se actualiza al volver a
  ella – otra pestaña, el otro panel o al volver a la ventana – si la lista tiene más de 5 segundos, y
  mientras la ventana está activa también cada 30 segundos (**Ver → Actualización automática**:
  desactivada, 10 s, 30 s, 1 min o 5 min; una carpeta con más de 20 000 elementos como mucho cada
  5 minutos). El cursor, las marcas, la posición de desplazamiento, el orden y los tamaños de carpeta
  calculados se mantienen. No se actualiza nada mientras cambia un nombre en la lista, arrastra o hay un
  diálogo abierto, y una conexión perdida no se reintenta. Tras sus propias operaciones se actualizan
  todas las pestañas que muestran la carpeta; **Ctrl+R** actualiza en el acto.

## Seleccionar archivos

- **Insert**, **Space** o **Ctrl+clic** marca un elemento; **Ctrl+A** marca todo o nada.
- **Shift+clic** marca los elementos desde el último pulsado o marcado hasta el que pulsa.
  **Shift** con **↑ ↓ PgUp PgDn Home End** marca desde allí hasta el cursor mientras se mueve;
  al volver atrás solo se desmarca lo que marcó este rango. **Herramientas → Seleccionar hasta el
  principio / Seleccionar hasta el final** hace lo mismo que **Shift+Home** y **Shift+End**. Un
  clic sin tecla mueve el cursor y conserva las marcas. El inicio del rango se vuelve a encontrar por
  su nombre tras una actualización, una ordenación o el filtro rápido; `..` nunca se marca. Arrastrar
  sigue copiando, y **Shift** pulsada al soltar los archivos los mueve (Shift+clic por sí solo nunca
  mueve nada).
- **Num+** marca un grupo según una máscara como `*.jpg;*.png`, **Alt+−** lo desmarca, **Alt+I**
  invierte la selección.
- Los diálogos de máscara de **Num+** y **Alt+−** recuerdan la última máscara y si
  **Incluir carpetas** está activado, cada uno por separado y también tras reiniciar. En un servidor
  marcado como de producción, **Incluir carpetas** siempre está desactivado al abrir el diálogo y lo
  que elija allí no se recuerda.
- La línea bajo la lista muestra cuántos elementos están marcados y su tamaño.
- **Alt+Space** (o **Shift+Space**) calcula el tamaño de la carpeta bajo el cursor o de las carpetas
  marcadas, **Alt+Shift+Enter** el de todas las carpetas del panel.

## Copiar, mover, renombrar

- **F5** copia los elementos marcados (o el que está bajo el cursor) a la carpeta del otro panel,
  **F6** los mueve. Ambos funcionan entre discos y servidores, también entre dos servidores.
- La carpeta de destino al copiar y mover se puede editar, elegir entre los destinos recientes o
  seleccionar con **Examinar…** (en el disco o en el servidor).
- Arrastre archivos con el ratón al otro panel, sobre una carpeta, sobre `..` o sobre una pestaña
  (manténgalos encima de una pestaña para cambiar a ella). Arrastrar copia, con **Shift** mueve, **Esc**
  cancela el arrastre. El diálogo Copiar o Mover siempre pregunta primero.
- Cuando un archivo ya existe en el destino, elija: omitir, sobrescribir solo los más antiguos,
  sobrescribir o reanudar una transferencia interrumpida. La opción predeterminada se puede
  establecer en el menú Configuración.
- Los archivos se escriben con un nombre temporal y se renombran cuando están completos, así que una
  transferencia interrumpida nunca deja un archivo a medias con el nombre real.
- Reanudar continúa solo un archivo parcial (`.filepart`) del mismo archivo de origen: su ruta,
  tamaño y fecha de modificación se guardan a su lado. Un archivo parcial de otro origen o de uno
  desconocido se deja intacto y se avisa; elija **Sobrescribir** para empezar de nuevo.
- Cuando el disco de destino se llena o se agota la cuota de disco, la transferencia se detiene y lo
  indica; un archivo anterior con el mismo nombre queda como estaba y se conserva la parte escrita.
  Libere espacio y pulse **Reintentar** en la cola de transferencias: la transferencia continúa donde
  se detuvo. Un servidor a menudo solo informa "Failure"; el programa consulta entonces el espacio
  libre en el servidor.
- Mover entre discos o servidores elimina un archivo de origen solo si no cambió durante la
  transferencia (tamaño y fecha de modificación) y la copia tiene el mismo tamaño. Con **Verificar
  con suma de comprobación antes de eliminar el origen** (activado por defecto al mover a un
  servidor) ambos se vuelven a leer y se comparan antes; una diferencia se avisa y el origen se
  queda.
- **F2** o **Shift+F6** renombra directamente en la lista: se selecciona el nombre sin la extensión,
  **Enter** confirma, **Esc** cancela.
- **F7** crea una carpeta, **F8** elimina (con confirmación). La eliminación se ejecuta en la cola
  de transferencias con el número de elementos eliminados; **Cancelar** la detiene (lo ya eliminado
  sigue eliminado).
- Las tuberías (FIFO), sockets, dispositivos y enlaces simbólicos que no llevan a ninguna parte
  (falta el destino o es un bucle) no se copian: se omiten y se listan en el informe final, el resto
  de la copia continúa.
- Los nombres inusuales se copian con exactitud: saltos de línea, barra invertida, nombres de
  255 bytes y, en los discos de este equipo, también nombres que no son UTF-8 válido. La lista los
  muestra en una línea: caracteres de control como `\n` o `\t`, cada byte no válido como `\xFF`. Un
  nombre que no es UTF-8 válido no se puede enviar a un servidor (SFTP toma los nombres como texto);
  se informa.
- La cola de transferencias ejecuta hasta tres transferencias a la vez. El límite de velocidad está
  en el diálogo de copia.
- **Qué conserva una copia** además del contenido: los permisos (lectura, escritura, ejecución; setuid,
  setgid y sticky se omiten) y la hora de modificación. No conserva el propietario ni el grupo (la copia
  pertenece al usuario que la escribe), las ACL, los atributos extendidos (tampoco las etiquetas de
  SELinux ni las capabilities de archivo) ni los enlaces duros (cada enlace se convierte en un archivo
  propio). SFTP no puede transmitir ACL ni atributos extendidos. Un archivo sobrescrito se sustituye por
  uno nuevo, así que también desaparecen sus propias ACL y atributos; el archivo nuevo recibe lo que la
  carpeta da a los archivos nuevos (por ejemplo, una ACL predeterminada).
- **Las horas** se muestran en la zona horaria de este equipo, también las de los archivos de un
  servidor, sea cual sea la zona horaria del servidor. **Sobrescribir solo los más antiguos** compara
  las horas al segundo e independientemente de las zonas horarias. Una copia conserva la hora de su
  origen, así que un reloj del servidor mal puesto no afecta a los archivos copiados; sí afecta a los
  archivos modificados en el servidor, que reciben la hora del servidor: si su reloj va una hora
  adelantado, un archivo modificado allí parece una hora más reciente.
- **Arrastrar a otras aplicaciones** (X11): arrastre archivos o carpetas fuera de la ventana, por ejemplo
  a un gestor de archivos o a un editor. Los archivos locales se copian, con **Shift** se mueven. Los
  archivos de un servidor o de un archivo comprimido solo se copian: una aplicación que admite el guardado
  directo (XDS) los recibe descargados en su carpeta mediante la cola de transferencias; si no, primero se
  descargan en una carpeta temporal privada (por encima de 1 GiB se pregunta), que se borra cuando ya nada
  la usa y al salir. Algunas aplicaciones esperan los archivos solo unos 30 segundos, así que una descarga
  grande puede no llegar por esta vía. De vuelta sobre la ventana, el arrastre continúa dentro; **Esc** lo
  cancela. No disponible en Wayland.

## Servidores

- **Ctrl+O** abre el gestor de servidores. Introduzca el servidor como `host` o `user@host:port`,
  guárdelo como perfil con un nombre y un color de pestaña, y conéctese con **Enter**.
- Marque un servidor como **servidor de producción**: sus pestañas tienen un encabezado rojo y las
  acciones peligrosas piden confirmación una vez más (la eliminación no se puede confirmar con
  Enter).
- En un servidor de producción tampoco se pueden confirmar con Enter la copia, la subida, el
  movimiento ni la creación de una carpeta, y la copia nunca empieza con Sobrescribir (está
  preseleccionado Omitir existentes).
- La clave del servidor se comprueba con `~/.ssh/known_hosts`. Una clave nueva solo se añade después
  de que usted confirme su huella; una clave cambiada se rechaza.
- El inicio de sesión prueba ssh-agent, sus claves y después la contraseña. Las contraseñas
  guardadas se conservan cifradas, protegidas por una contraseña maestra.
- Las contraseñas guardadas se vuelven a bloquear un tiempo ajustable después de su último uso (menú
  Configuración → contraseña maestra, 60 segundos por defecto); la barra de estado muestra el tiempo
  restante. Al bloquear también se olvidan las contraseñas guardadas para reconectar.
- Si se pierde la conexión, RolidSCP vuelve a conectarse solo y las transferencias interrumpidas
  continúan. Una transferencia sigue mientras avance entre los cortes; tras cinco cortes seguidos
  sin avanzar se detiene y puede iniciarla de nuevo desde la cola.
- Si no llega nada del servidor durante 15 segundos mientras el programa lo espera, la conexión se
  considera muerta y se vuelve a establecer enseguida (una breve comprobación a los 5 segundos
  distingue un servidor lento de una línea muerta). En los tres primeros minutos tras un corte se
  intenta cada pocos segundos, luego cada 10 segundos y, pasados diez minutos, cada 30 segundos. Una
  subida continúa desde la última parte que el servidor confirmó.
- Lo mismo vale cuando el cable de red está desconectado durante minutos o el servidor se suspende:
  el programa sigue intentándolo y la transferencia continúa en cuanto el servidor vuelve a estar
  accesible.
- En un servidor sin shell o sin algunas herramientas (`du`, `find`, `df`, `sha256sum`, por ejemplo
  BusyBox o una cuenta solo SFTP) todo se hace solo por SFTP: los tamaños de carpeta y la búsqueda
  recorriendo las carpetas, las sumas de comprobación leyendo los archivos.
- Un servidor defectuoso o malicioso no puede hacer que el programa escriba fuera de la carpeta de
  destino: los nombres con `/`, `..` o vacíos se rechazan y los caracteres de control de los nombres
  se muestran como `\xNN`. Un listado de carpeta de más de 5 millones de elementos se detiene, y un
  archivo que el servidor envía mucho más allá de su tamaño no se acepta.
- **Ctrl+P** abre el terminal del sistema (`$TERMINAL` o `x-terminal-emulator`) en la carpeta del
  panel. En un servidor ejecuta `ssh` a esa carpeta, con la misma comprobación de la clave del
  servidor que RolidSCP (una clave desconocida o cambiada termina la conexión). Una contraseña
  guardada la pasa un ayudante de un solo uso (`SSH_ASKPASS`) que solo responde a la pregunta por la
  contraseña de ese servidor, una vez; si las contraseñas están bloqueadas, primero se pide la
  contraseña maestra; si no, ssh la pide él mismo. En un servidor de producción se pide confirmación
  antes. La contraseña nunca pasa por una línea de comandos, un archivo ni el registro.
- **Ctrl+L** devuelve el panel activo al disco local.

## Ver archivos (F3)

- **F3** abre el archivo en el visor, una ventana propia (se pueden abrir varias a la vez). También
  está en el menú contextual, en el menú Archivo y en la barra de teclas. El visor nunca ejecuta nada
  ni carga nada de fuera: HTML, XML y SVG se muestran como texto.
- Los archivos de cualquier tamaño se abren al instante: solo se lee la parte que está en pantalla. Un
  archivo de un servidor no se descarga entero; un archivo dentro de un comprimido se extrae primero a
  un archivo temporal. Los números de línea con **~** son estimados hasta que se cuentan las líneas.
- **1**, **2** y **3** cambian entre texto, hexadecimal e imagen; los archivos binarios se muestran en
  hexadecimal por sí solos.
- **W** ajusta las líneas, **N** muestra los números de línea, **Ctrl+G** va a una línea, un porcentaje
  o una posición, **Ctrl+Home** y **Ctrl+End** van al principio y al final. Los menús Codificación y
  Resaltado cambian la codificación y el lenguaje del resaltado de sintaxis (desactivado en archivos
  de más de 20 MB, véase **Configuración → Configuración… → Visor (F3)**).
- **F** sigue el final de un archivo que crece, también en un servidor; desplazarse hacia arriba lo
  pausa. Cuando el archivo cambia, una barra ofrece recargarlo. Una conexión perdida se indica con una
  barra y la vista continúa sola cuando vuelve.
- **Ctrl+F** o **F7** busca texto (mayúsculas, palabras completas, expresión regular); **F3** y
  **Shift+F3** van a la coincidencia siguiente y anterior. Se busca en todo el archivo en segundo
  plano; en un servidor busca allí `grep` cuando es posible y, si no, el archivo se lee por SFTP; la
  barra de búsqueda indica cuál.
- Se selecciona con el ratón o con Shift y las flechas; **Ctrl+C** copia la selección (32 MB como
  máximo).
- Imágenes (JPEG, PNG, GIF también animado, WebP, BMP): **F** ajusta la imagen a la ventana, **0** la
  muestra al 100 %, la rueda hace zoom, arrastrar la mueve, **R** y **L** giran la vista, las flechas
  y **PgUp**/**PgDn** muestran la imagen anterior y la siguiente de la carpeta. Una imagen de un
  servidor solo se descarga hasta 50 MB (ajustable).
- **F4** edita el archivo como en el panel; **Esc** o **Q** cierra el visor.

## Editar y abrir archivos

- **F4** abre el archivo en un editor externo. Un archivo de un servidor se vuelve a subir tras cada
  guardado; si mientras tanto ha cambiado en el servidor, se le pregunta qué hacer.
- La copia local se guarda en una carpeta privada en el disco (`~/.local/state/rolidscp/edit`, solo
  para usted) y se elimina al terminar la edición. Una subida que falla por otro motivo que una
  conexión perdida se reintenta dos veces y luego se avisa. La hora de la última subida se muestra
  con segundos.
- Un servidor sin cambio de nombre atómico recibe un archivo modificado por pasos (el original se
  aparta primero). Si la conexión o el programa se interrumpen entre medias, el paso se completa o
  se deshace en la siguiente conexión a ese servidor, y se le indica a qué archivos afectó.
- **Shift+F4** (también **Archivo → Archivo nuevo…** y el menú contextual) crea un archivo nuevo y lo
  abre en el editor externo. Se propone el nombre bajo el cursor. En este equipo el archivo vacío se crea
  enseguida; en un servidor se crea al guardarlo por primera vez, así que cerrar el editor sin guardar no
  crea nada. Un nombre existente abre ese archivo para editarlo.
- **Shift+Enter** o el menú contextual (clic derecho) ofrecen **Abrir con…**, los permisos
  (**Ctrl+Shift+A**) y copiar el nombre, la ruta o la dirección `sftp://` al portapapeles.
- Después de guardar en el editor (**F4**, **Shift+F4**), el tamaño y la fecha de un archivo de este
  equipo se actualizan en el panel.

## Archivos comprimidos

- **Alt+F5** comprime los elementos marcados en zip, 7z, tar, tar.gz o tar.zst, opcionalmente con
  contraseña (zip y 7z).
- **Alt+F9** extrae un archivo comprimido en el otro panel, con protección contra rutas peligrosas y
  bombas de compresión. **Alt+Shift+F9** prueba un archivo comprimido.
- Un empaquetado o una extracción interrumpidos por un fallo o por matar el programa dejan su
  archivo `.filepart` sin terminar solo hasta el siguiente inicio del programa, que lo borra.
- **Enter** sobre un archivo comprimido lo abre como carpeta de solo lectura; **F5** copia archivos
  fuera de él.
- Las horas en un zip se guardan al segundo y en UTC (como hace Info-ZIP), así que son correctas también
  en otra zona horaria. Un zip de un programa que solo escribe la antigua hora DOS (sin zona horaria,
  solo segundos pares) se lee en la zona horaria de este equipo.

## Búsqueda y sumas de comprobación

- **Ctrl+F7** (o **Ctrl+F**) busca por nombre (máscara, texto o expresión regular) y por contenido.
  En un servidor la búsqueda se ejecuta allí con `find` y `grep` cuando es posible.
- Las sumas de comprobación (MD5, SHA-1, SHA-256, SHA-512, BLAKE3, CRC32) están en el menú Archivo y
  en el menú contextual; se pueden guardar en el formato de `sha256sum -c`.

## Estado del disco (SMART)

- La insignia de la fila de la ruta muestra el estado del disco: **OK**, **aviso** (naranja) o
  **crítico** (rojo). Un clic muestra una explicación clara y una recomendación.
- Los detalles muestran el modelo y el número de serie, el tipo (HDD, SSD, NVMe), el desgaste y la
  vida restante estimada, la temperatura, el tiempo de funcionamiento, los encendidos, los apagados
  inesperados, los sectores defectuosos e inestables, los errores de transferencia (CRC; normalmente
  un cable o un conector) y los datos escritos. Todos los atributos están en la sección Avanzado.
- Los datos vienen de UDisks2 (sin permisos de administrador) o de `smartctl` cuando funciona sin
  ellos. Se leen en segundo plano, como mucho una vez al día por disco (guardados en
  `~/.cache/rolidscp/smart.json`); **Volver a cargar** lee el disco ahora.
- Aviso: desgaste de un SSD a partir del 60 %, cualquier sector defectuoso, inestable o ilegible, un
  disco duro a partir de 45 °C o un SSD a partir de 60 °C. Crítico: desgaste a partir del 95 %, más
  de 50 sectores defectuosos o un número creciente, un disco duro a partir de 55 °C o un SSD a partir
  de 75 °C, un aviso crítico de NVMe o poca reserva, una autoprueba fallida. La temperatura se
  comprueba cada 5 minutos.
- Los sistemas de archivos de red, los discos virtuales y los adaptadores USB sin SMART tienen una
  insignia gris; su descripción y los detalles dicen por qué.
- La insignia es un pequeño icono de pulso: gris cuando el estado no se conoce, verde cuando el disco
  está bien, naranja o rojo con el motivo más grave al lado (por ejemplo «Desgaste 87 %», «Sectores
  defectuosos» o «61 °C»). Al pasar el ratón muestra todos los motivos. La barra del espacio libre se
  vuelve naranja por debajo del 10 % libre y roja por debajo del 5 %.

## Configuración

- **Configuración → Configuración…** contiene el idioma, el formato de fecha, la fuente (integrada o
  del sistema), el tamaño del texto, el hinting y el contraste, la barra de herramientas, los
  colores por tipo de archivo, las miniaturas y los límites de los archivos comprimidos.
- De forma predeterminada el idioma sigue al sistema (`LC_ALL`, `LC_MESSAGES`, `LANG`). Se puede
  elegir en Configuración y cambia al instante. Su propia traducción
  `~/.config/rolidscp/lang/<language>.po` se usa en lugar de la integrada.

## Archivos y registro

- Configuración: `~/.config/rolidscp/` (legible solo por usted).
- Registro: `~/.local/state/rolidscp/rolidscp.log` (menú Ayuda → Abrir registro (diagnóstico de
  inicio de sesión)). Nunca contiene contraseñas.
- El AppImage se puede actualizar con AppImageUpdate (o `appimageupdatetool
  RolidSCP-<versión>-x86_64.AppImage`): encuentra la versión más reciente en GitHub y descarga solo
  las partes que han cambiado. El nuevo AppImage se guarda junto al anterior con el nombre de la
  versión; el anterior se conserva.
