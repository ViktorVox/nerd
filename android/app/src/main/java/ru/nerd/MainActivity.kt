package ru.nerd

import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.BackHandler
import androidx.activity.compose.setContent
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.gestures.detectTapGestures
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.material.icons.rounded.Add
import androidx.compose.material.icons.rounded.KeyboardArrowUp
import androidx.compose.material.icons.rounded.Menu
import androidx.compose.material.icons.rounded.Redo
import androidx.compose.material.icons.rounded.Remove
import androidx.compose.material.icons.rounded.RotateLeft
import androidx.compose.material.icons.rounded.RotateRight
import androidx.compose.material.icons.rounded.Settings
import androidx.compose.material.icons.rounded.Undo
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.FilterChip
import androidx.compose.material3.Slider
import androidx.compose.ui.draw.clip
import androidx.compose.ui.input.pointer.PointerEventPass
import androidx.compose.ui.input.pointer.PointerEventType
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.Close
import androidx.compose.material.icons.rounded.Refresh
import androidx.compose.material3.Button
import androidx.compose.material3.Card
import androidx.compose.material3.CardDefaults
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.SegmentedButton
import androidx.compose.material3.SegmentedButtonDefaults
import androidx.compose.material3.SingleChoiceSegmentedButtonRow
import androidx.compose.material3.Surface
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.darkColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.compose.ui.viewinterop.AndroidView
import androidx.core.view.WindowCompat
import androidx.core.view.WindowInsetsCompat
import androidx.core.view.WindowInsetsControllerCompat
import kotlinx.coroutines.delay

private val NerdColors = darkColorScheme(
    primary = Color(0xFF8FA8FF),
    onPrimary = Color(0xFF0B1330),
    background = Color(0xFF0E0F13),
    onBackground = Color(0xFFE6E8EE),
    surface = Color(0xFF181A21),
    onSurface = Color(0xFFE6E8EE),
    onSurfaceVariant = Color(0xFF9AA0B0),
    secondaryContainer = Color(0xFF2B3252),
    onSecondaryContainer = Color(0xFFDDE3FF),
    outline = Color(0xFF3A3E4D),
)

class MainActivity : ComponentActivity() {
    private val direct by lazy { DirectNetwork(this) }

    override fun onDestroy() {
        direct.stop()
        super.onDestroy()
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        direct.start()
        WindowCompat.setDecorFitsSystemWindows(window, false)
        WindowInsetsControllerCompat(window, window.decorView).apply {
            hide(WindowInsetsCompat.Type.systemBars())
            systemBarsBehavior = WindowInsetsControllerCompat.BEHAVIOR_SHOW_TRANSIENT_BARS_BY_SWIPE
        }
        val prefs = getSharedPreferences("nerd", MODE_PRIVATE)
        val identity = Identity.load(prefs)
        AppSettings.load(prefs)

        setContent {
            MaterialTheme(colorScheme = NerdColors) {
                Surface(modifier = Modifier.fillMaxSize(), color = MaterialTheme.colorScheme.background) {
                    var host by remember { mutableStateOf<String?>(null) }
                    // Сразу подключаемся к последнему компьютеру, но не после того, как пользователь сам отключился
                    var auto by remember { mutableStateOf(true) }
                    val current = host
                    if (current == null) {
                        ConnectScreen(last = prefs.getString("ip", "") ?: "", auto = auto) {
                            prefs.edit().putString("ip", it).apply()
                            host = it
                        }
                    } else {
                        PadScreen(current, identity) { auto = false; host = null }
                    }
                }
            }
        }
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun ConnectScreen(last: String, auto: Boolean, onConnect: (String) -> Unit) {
    val context = androidx.compose.ui.platform.LocalContext.current
    val discovery = remember { Discovery(context) }
    DisposableEffect(Unit) { onDispose { discovery.stop() } }

    var found by remember { mutableStateOf(emptyList<FoundPc>()) }
    var manual by remember { mutableStateOf(false) }
    var ip by remember { mutableStateOf(last) }
    LaunchedEffect(Unit) {
        while (true) {
            found = discovery.found
            delay(500)
        }
    }
    LaunchedEffect(found) {
        if (auto && last.isNotEmpty() && found.any { it.ip == last }) onConnect(last)
    }

    Box(modifier = Modifier.fillMaxSize().statusBarsPadding(), contentAlignment = Alignment.Center) {
        Column(
            modifier = Modifier.widthIn(max = 460.dp).fillMaxWidth().padding(24.dp),
            horizontalAlignment = Alignment.CenterHorizontally,
            verticalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            Text("Nerd", style = MaterialTheme.typography.displayMedium, color = MaterialTheme.colorScheme.onBackground)
            Text(
                "Превращает устройство в графический планшет для компьютера",
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
            Spacer(Modifier.height(12.dp))

            if (found.isEmpty()) {
                Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                    CircularProgressIndicator(modifier = Modifier.size(20.dp), strokeWidth = 2.dp)
                    Text(
                        "Ищу компьютеры в сети…",
                        style = MaterialTheme.typography.bodyMedium,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
                Text(
                    "Запусти программу Nerd на компьютере. Устройство и компьютер должны быть в одной Wi-Fi сети.",
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    textAlign = TextAlign.Center,
                )
            }
            found.forEach { pc ->
                Card(
                    onClick = { onConnect(pc.ip) },
                    shape = RoundedCornerShape(20.dp),
                    colors = CardDefaults.cardColors(containerColor = MaterialTheme.colorScheme.surface),
                    modifier = Modifier.fillMaxWidth(),
                ) {
                    Column(modifier = Modifier.padding(horizontal = 20.dp, vertical = 16.dp)) {
                        Text(pc.name, style = MaterialTheme.typography.titleMedium, color = MaterialTheme.colorScheme.onSurface)
                        Text(pc.ip, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
                    }
                }
            }

            TextButton(onClick = { manual = !manual }) {
                Text(if (manual) "Скрыть ввод адреса" else "Ввести адрес вручную")
            }
            if (manual) {
                OutlinedTextField(
                    value = ip,
                    onValueChange = { ip = it },
                    label = { Text("IP компьютера") },
                    placeholder = { Text("192.168.1.10") },
                    singleLine = true,
                    keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Uri),
                    shape = RoundedCornerShape(16.dp),
                    modifier = Modifier.fillMaxWidth(),
                )
                Button(
                    onClick = { if (ip.isNotBlank()) onConnect(ip.trim()) },
                    shape = RoundedCornerShape(16.dp),
                    modifier = Modifier.fillMaxWidth().height(52.dp),
                ) { Text("Подключиться") }
            }
        }
    }
}

@OptIn(ExperimentalMaterial3Api::class)
// Нажатие в этом месте сообщается, но не перехватывается
private fun Modifier.onPress(action: () -> Unit): Modifier = pointerInput(Unit) {
    awaitPointerEventScope {
        while (true) {
            val e = awaitPointerEvent(PointerEventPass.Initial)
            if (e.type == PointerEventType.Press) action()
        }
    }
}

@Composable
private fun PadScreen(host: String, identity: Identity, onExit: () -> Unit) {
    val context = androidx.compose.ui.platform.LocalContext.current
    val sender = remember { Sender(host) }
    val pad = remember { PadView(context, sender) }
    val video = remember { android.view.TextureView(context).also { pad.attachVideo(it) } }
    val client = remember { ScreenClient(context, host, pad, sender, identity) }
    DisposableEffect(Unit) { onDispose { client.stop() } }
    BackHandler(onBack = onExit)

    var mode by remember { mutableStateOf(pad.mode) }
    val prefsQuick = remember { context.getSharedPreferences("nerd", android.content.Context.MODE_PRIVATE) }
    var fingerPressure by remember { mutableStateOf(AppSettings.fingerPressure) }
    var stats by remember { mutableStateOf("") }
    var state by remember { mutableStateOf(ConnState.CONNECTING) }
    var problem by remember { mutableStateOf("") }
    var zoom by remember { mutableStateOf(1f) }
    var monitors by remember { mutableStateOf(emptyList<MonitorOption>()) }
    var currentMonitor by remember { mutableStateOf(0) }
    var menuOpen by remember { mutableStateOf(false) }
    var showSettings by remember { mutableStateOf(false) }
    var panelsVisible by remember { mutableStateOf(true) }
    var panelTouches by remember { mutableStateOf(0) }
    var hideAfter by remember { mutableStateOf(AppSettings.panelHideSeconds) }
    LaunchedEffect(panelsVisible, panelTouches, hideAfter, showSettings, menuOpen) {
        if (panelsVisible && hideAfter > 0 && !showSettings && !menuOpen) {
            delay(hideAfter * 1000L)
            panelsVisible = false
        }
    }
    var ctrl by remember { mutableStateOf(false) }
    var shift by remember { mutableStateOf(false) }
    var spaceDown by remember { mutableStateOf(false) }
    // Залипшие Ctrl и Shift отпускаем при выходе, чтобы они не остались нажатыми на компьютере
    DisposableEffect(Unit) {
        onDispose {
            if (ctrl) sender.key(VK_CTRL, false)
            if (shift) sender.key(VK_SHIFT, false)
        }
    }
    LaunchedEffect(Unit) {
        while (true) {
            stats = client.statsText
            state = client.state
            problem = client.problem
            zoom = pad.zoom
            monitors = client.monitors
            currentMonitor = client.currentMonitor
            delay(400)
        }
    }

    Box(modifier = Modifier.fillMaxSize().background(Color(0xFF0E0F13))) {
        AndroidView(factory = { video }, modifier = Modifier.fillMaxSize())
        AndroidView(factory = { pad }, modifier = Modifier.fillMaxSize())

        if (state != ConnState.CONNECTED) {
            Column(
                modifier = Modifier.align(Alignment.Center).padding(24.dp),
                horizontalAlignment = Alignment.CenterHorizontally,
                verticalArrangement = Arrangement.spacedBy(12.dp),
            ) {
                when (state) {
                    ConnState.DENIED -> {
                        Text("Компьютер отклонил подключение", color = MaterialTheme.colorScheme.onBackground, style = MaterialTheme.typography.titleMedium)
                        Text(
                            "Попробуй снова и нажми «Разрешить» в окне на компьютере.",
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                            textAlign = TextAlign.Center,
                        )
                        Button(onClick = onExit, shape = RoundedCornerShape(16.dp)) { Text("Назад") }
                    }
                    ConnState.WAITING -> {
                        CircularProgressIndicator(color = MaterialTheme.colorScheme.primary)
                        Text("Подтвердите подключение на компьютере", color = MaterialTheme.colorScheme.onBackground, style = MaterialTheme.typography.titleMedium)
                        Text(
                            "На экране компьютера появилось окно «Разрешить / Отклонить».",
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                            textAlign = TextAlign.Center,
                        )
                    }
                    else -> {
                        CircularProgressIndicator(color = MaterialTheme.colorScheme.primary)
                        Text("Подключение к $host…", color = MaterialTheme.colorScheme.onSurfaceVariant)
                        if (problem.isNotEmpty()) {
                            Text(problem, color = MaterialTheme.colorScheme.onBackground, textAlign = TextAlign.Center)
                        }
                    }
                }
            }
        }

        if (panelsVisible) {
            Surface(
                shape = RoundedCornerShape(28.dp),
                color = MaterialTheme.colorScheme.surface.copy(alpha = 0.88f),
                tonalElevation = 6.dp,
                modifier = Modifier.align(Alignment.TopCenter).statusBarsPadding().padding(top = 8.dp).onPress { panelTouches++ },
            ) {
                Row(
                    modifier = Modifier.horizontalScroll(rememberScrollState()).padding(horizontal = 8.dp, vertical = 4.dp),
                    verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.spacedBy(4.dp),
                ) {
                    SingleChoiceSegmentedButtonRow {
                        Mode.entries.forEachIndexed { index, m ->
                            SegmentedButton(
                                selected = mode == m,
                                onClick = { mode = m; pad.mode = m },
                                shape = SegmentedButtonDefaults.itemShape(index, Mode.entries.size),
                            ) { Text(m.title) }
                        }
                    }
                    if (mode == Mode.DRAW) {
                        TextButton(onClick = {
                            val next = Math.round(fingerPressure * 5) % 5 + 1
                            fingerPressure = next / 5f
                            AppSettings.fingerPressure = fingerPressure
                            AppSettings.save(prefsQuick)
                        }) { Text("Нажим ${(fingerPressure * 100).toInt()}%") }
                    }
                    IconButton(onClick = { pad.fit() }) {
                        Icon(Icons.Rounded.Refresh, contentDescription = "Сбросить вид")
                    }
                    IconButton(onClick = { showSettings = true }) {
                        Icon(Icons.Rounded.Settings, contentDescription = "Настройки")
                    }
                    if (monitors.size > 1) {
                        Box {
                            TextButton(onClick = { menuOpen = true }) { Text("Монитор $currentMonitor ▾") }
                            DropdownMenu(expanded = menuOpen, onDismissRequest = { menuOpen = false }) {
                                monitors.forEach { m ->
                                    DropdownMenuItem(
                                        text = { Text("Монитор ${m.num} · ${m.w}×${m.h}") },
                                        onClick = { menuOpen = false; client.selectMonitor(m.num) },
                                    )
                                }
                            }
                        }
                    }
                    if (state == ConnState.CONNECTED) {
                        Text(
                            "×%.1f".format(zoom) + if (stats.isNotEmpty()) " · $stats" else "",
                            style = MaterialTheme.typography.labelMedium,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                            modifier = Modifier.padding(horizontal = 4.dp),
                        )
                    }
                    IconButton(onClick = { panelsVisible = false }) {
                        Icon(Icons.Rounded.KeyboardArrowUp, contentDescription = "Скрыть панели")
                    }
                    IconButton(onClick = onExit) {
                        Icon(Icons.Rounded.Close, contentDescription = "Отключиться")
                    }
                }
            }

            // Нижняя панель: быстрые клавиши для Krita и других программ рисования
            Surface(
                shape = RoundedCornerShape(28.dp),
                color = MaterialTheme.colorScheme.surface.copy(alpha = 0.88f),
                tonalElevation = 6.dp,
                modifier = Modifier.align(Alignment.BottomCenter).navigationBarsPadding().padding(bottom = 8.dp, start = 8.dp, end = 8.dp).onPress { panelTouches++ },
            ) {
                Row(
                    modifier = Modifier.horizontalScroll(rememberScrollState()).padding(horizontal = 8.dp, vertical = 4.dp),
                    verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.spacedBy(2.dp),
                ) {
                    IconButton(onClick = { sender.undo() }) { Icon(Icons.Rounded.Undo, contentDescription = "Отменить") }
                    IconButton(onClick = { sender.redo() }) { Icon(Icons.Rounded.Redo, contentDescription = "Вернуть") }
                    Spacer(Modifier.width(6.dp))
                    IconButton(onClick = { sender.combo(VK_BRACKET_LEFT) }) { Icon(Icons.Rounded.Remove, contentDescription = "Кисть меньше") }
                    IconButton(onClick = { sender.combo(VK_BRACKET_RIGHT) }) { Icon(Icons.Rounded.Add, contentDescription = "Кисть больше") }
                    TextButton(onClick = { sender.combo(VK_E) }) { Text("Ластик") }
                    Spacer(Modifier.width(6.dp))
                    IconButton(onClick = { sender.rotateCanvas(false) }) { Icon(Icons.Rounded.RotateLeft, contentDescription = "Повернуть холст влево") }
                    IconButton(onClick = { sender.rotateCanvas(true) }) { Icon(Icons.Rounded.RotateRight, contentDescription = "Повернуть холст вправо") }
                    Spacer(Modifier.width(6.dp))
                    FilterChip(
                        selected = ctrl,
                        onClick = { ctrl = !ctrl; sender.key(VK_CTRL, ctrl) },
                        label = { Text("Ctrl") },
                    )
                    FilterChip(
                        selected = shift,
                        onClick = { shift = !shift; sender.key(VK_SHIFT, shift) },
                        label = { Text("Shift") },
                    )
                    // Пробел работает, пока кнопка удерживается: так двигают холст в Krita
                    val chip = RoundedCornerShape(8.dp)
                    Box(
                        modifier = Modifier
                            .height(32.dp)
                            .clip(chip)
                            .background(if (spaceDown) MaterialTheme.colorScheme.secondaryContainer else Color.Transparent)
                            .border(1.dp, MaterialTheme.colorScheme.outline, chip)
                            .padding(horizontal = 14.dp)
                            .pointerInput(Unit) {
                                detectTapGestures(onPress = {
                                    spaceDown = true
                                    sender.key(VK_SPACE, true)
                                    try { awaitRelease() } finally {
                                        spaceDown = false
                                        sender.key(VK_SPACE, false)
                                    }
                                })
                            },
                        contentAlignment = Alignment.Center,
                    ) { Text("Пробел", style = MaterialTheme.typography.labelLarge) }
                }
            }
        }

        if (!panelsVisible) {
            Surface(
                shape = RoundedCornerShape(50),
                color = MaterialTheme.colorScheme.surface.copy(alpha = 0.55f),
                modifier = Modifier.align(Alignment.TopEnd).statusBarsPadding().padding(8.dp),
            ) {
                IconButton(onClick = { panelsVisible = true }) {
                    Icon(Icons.Rounded.Menu, contentDescription = "Показать панели")
                }
            }
        }

        if (showSettings) {
            val prefs = remember { context.getSharedPreferences("nerd", android.content.Context.MODE_PRIVATE) }
            var quality by remember { mutableStateOf(AppSettings.quality) }
            var pressure by remember { mutableStateOf(AppSettings.fingerPressure) }
            var useVideo by remember { mutableStateOf(AppSettings.video) }
            var hide by remember { mutableStateOf(AppSettings.panelHideSeconds) }
            AlertDialog(
                onDismissRequest = { showSettings = false },
                title = { Text("Настройки") },
                text = {
                    Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
                        Text("Качество картинки: $quality", style = MaterialTheme.typography.bodyMedium)
                        Slider(
                            value = quality.toFloat(),
                            onValueChange = { quality = it.toInt(); AppSettings.quality = quality },
                            valueRange = 30f..90f,
                        )
                        Text("Сила нажима пальцем: ${(pressure * 100).toInt()}%", style = MaterialTheme.typography.bodyMedium)
                        Slider(
                            value = pressure,
                            onValueChange = { pressure = it; AppSettings.fingerPressure = it },
                            valueRange = 0.1f..1f,
                        )
                        Row(verticalAlignment = Alignment.CenterVertically) {
                            Text("Видео H.264 (меньше трафика и задержки). Применится при следующем подключении.", style = MaterialTheme.typography.bodyMedium, modifier = Modifier.weight(1f))
                            Switch(checked = useVideo, onCheckedChange = { useVideo = it; AppSettings.video = it })
                        }
                        Text(
                            if (hide == 0) "Скрывать панели: никогда" else "Скрывать панели через: $hide с",
                            style = MaterialTheme.typography.bodyMedium,
                        )
                        Slider(
                            value = hide.toFloat(),
                            onValueChange = { hide = it.toInt(); AppSettings.panelHideSeconds = hide },
                            valueRange = 0f..30f,
                            steps = 29,
                        )
                    }
                },
                confirmButton = {
                    TextButton(onClick = { AppSettings.save(prefs); fingerPressure = AppSettings.fingerPressure; hideAfter = AppSettings.panelHideSeconds; showSettings = false }) { Text("Готово") }
                },
            )
        }
    }
}
