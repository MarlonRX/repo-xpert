# VISION — git-advance

> Nombre en clave. El público se decide al final de M5.

## Qué es

Un **motor de analítica de repositorios Git con TUI**, escrito en Rust, que
responde preguntas que `lazygit`/`gitui`/`tig` no responden:

- *¿Qué archivos cambian constantemente y cuáles son un polvorín?* (churn, hotspots)
- *¿Quién es dueño de qué código? ¿Cuánta gente se tiene que ir para que un módulo quede huérfano?* (ownership, bus factor)
- *¿Qué archivos cambian siempre juntos aunque no se importen entre sí?* (coupling)

Tres atributos no negociables:

1. **Local y de solo lectura.** No toca red, no escribe en el working tree, no
   necesita credenciales. Abre `.git`, lee objetos, calcula, muestra.
2. **Motor propio.** El historial se recorre leyendo los objetos Git con una
   librería nativa (`gix`, a confirmar en DECISIONS §1), no shellando a `git`.
   El cómputo es una función pura `historial → métricas`.
3. **Caché incremental.** Un repo de 10k commits se indexa una vez; abrir la
   herramienta después es cargar un archivo, no recalcular.

## Qué NO es

- **No es un cliente de operaciones Git.** Nada de stage, commit, push, pull,
  stash, merge, resolución de conflictos. Todo eso sigue siendo de git-hero.
- **No es un dashboard de proceso (DORA).** No mide lead time ni frecuencia de deploy.
- **No es un SaaS ni tiene servidor, cuentas ni telemetría.**
- **No es multi-repo (aún).** Un repo por ejecución.
- **No es `git-hero` mejorado.** Es un producto hermano que comparte código
  heredado (esqueleto TUI, themes, i18n) pero no destino.

## Relación con git-hero

git-hero (el fork originario) **se mantiene como el gestor de repos**: el
"otro lazygit", con operaciones, credenciales, askpass y empaquetado.
git-advance **corta el cordón**: elimina toda la superficie de mutación
(`git.rs` operativo, askpass, modales de push/pull, CLI de operaciones) y
no intenta ser retrocompatible con ella.

Regla de decisión explícita: *este fork muere si no aporta analítica*. Si al
final de M3 (hotspots con scatter) el motor no corre claramente más que
`git log -p` + awk sobre un repo de 10k commits, el proyecto se cierra sin
pena y se vuelve a git-hero. La analítica es el producto; el TUI es su cara.

Los dos proyectos pueden compartir en el futuro un crate `gadv-engine`
(publish en crates.io) pero **eso es post-M5 y no bloquea nada ahora**.

## Público

- **Primary:** desarrolladores individuales que quieren entender un repo ajeno
  o el propio antes de tocar código (onboarding, auditoría, tech debt).
- **Secondary:** leads/staff engineers que necesitan justificar refactor con
  datos (hotspots, bus factor) sin pagar CodeScene.
- **Terciario (el objetivo real):** entrevistadores técnicos. Es un proyecto
  de portafolio: demuestra Rust de verdad — ownership, traits, errores
  tipados, estructuras de datos (grafos, índices internados), algoritmos y
  paralelismo con rayon — sobre una base académica real (Tornhill).

## El "wow" de portafolio

Abres tu repo más grande, pulsas una tecla y en <2 s ves:

- barras de churn top-20,
- un scatter churn×complejidad con tus polvorines en la esquina superior derecha,
- el bus factor de cada módulo en rojo cuando es 1,
- el grafo de co-modificación de un archivo seleccionado.

Ninguna herramienta gratuita y local hace eso hoy. El mercado de *clientes TUI
de Git* está saturado; el de *analítica TUI de Git* está vacío porque nadie
puede fingirlo: o tienes un motor, o no lo tienes.

## RIESGOS DETECTADOS (en el anteproyecto y en el terreno)

No se corrigieron en silencio; se listan para que el autor los valide:

1. **El anteproyecto se contradice con su base.** Exige "no depender del
   binario git" (§1, §3.1) y a la vez propone "reutilizar la base de Git
   Hero", cuyo corazón (`src/git.rs`, 900 líneas) es 100 % subprocesos `git`.
   Nadie dijo qué se conserva; en la práctica solo sirve el esqueleto TUI.
   → Se resuelve con el corte de cordón de DECISIONS §2.
2. **Promesa de alcance inflada.** El MVP del anteproyecto (§3.3) pide *seis
   métricas + ventanas + normalización + drill-down + export JSON/CSV* en
   7 milestones sin contemplar **caché**, que es el requisito duro de que la
   analítica no se recalcule por frame. → ROADMAP reordena: la caché entra
   en M2 (temprano) y *activity, staleness y export salen* del alcance
   M0–M5. Es una reducción explícita, no un olvido.
3. **Ownership "por blame" es el riesgo #1 de cronograma.** El anteproyecto
   (§5.2) define ownership con `blame`; blamear 10k commits en repos grandes
   es caro y el blame de `gix` es API joven. Un novato en Rust puede
   reventar ahí el hito M4. → ALGORITHMS §3 especifica ownership por commits
   (heurística) para M4 y deja blame como mejora post-M5 documentada.
4. **`trait Metric` del anteproyecto (§6.3) es fricción prematura.** Con
   `Output` asociado distinto por métrica no hay dyn-compatibilidad; el trait
   solo aporta después de tener 3 métricas estables. → se pospone (DECISIONS §3).
5. **`chrono` listado como "nuevo" (§6.2) es innecesario.** Git entrega
   epoch seconds (`i64`); los buckets temporales se hacen con aritmética
   entera. Menos una dependencia y menos superficie de aprendizaje.
6. **El "24 % de un frame" del plan de refactor (§1.1 de MAJOR_REFACTOR_PLAN)
   es una estimación sin medir.** Ningún número de rendimiento del anteproyecto
   está verificado. ROADMAP exige medir con cronómetro de pared en cada hito.
7. **Estado operativo:** la carpeta `D:\projects\git-advance` está **vacía**;
   el código del fork todavía no fue copiado de git-hero. M0 arranca recién
   de que eso se concrete (es tarea cero del ROADMAP, no cuenta en las 4 h).
8. **Renombre de identidad.** El crate, binario y paths de config/heredados
   dicen `gith`/`git-hero` (Cargo.toml, askpass, update-check). Si el corte
   es "solo lectura", gran parte de eso se borra y el problema desaparece
   por sí solo; pero si el autor eligiera modo híbrido (DECISIONS §2B),
   arrastraría lógica de credenciales a un producto que promete no escribir.
