package com.opendroidmic

import android.Manifest
import android.app.Activity
import android.content.ComponentName
import android.content.Intent
import android.content.ServiceConnection
import android.content.pm.PackageManager
import android.os.Build
import android.os.Bundle
import android.os.Handler
import android.os.IBinder
import android.os.Looper
import android.view.View
import android.widget.ProgressBar
import android.widget.TextView
import android.widget.Toast
import androidx.activity.result.contract.ActivityResultContracts
import androidx.appcompat.app.AppCompatActivity
import androidx.core.app.ActivityCompat
import androidx.core.content.ContextCompat
import com.google.android.material.button.MaterialButton

class MainActivity : AppCompatActivity() {
    companion object {
        private const val PERM_RECORD_AUDIO = 100
        private const val PERM_CAMERA = 101
        private const val PERM_DISCOVERY = 102
        private const val UI_UPDATE_INTERVAL = 100L
        private const val PREFS_NAME = "opendroidmic"
        private const val KEY_HOST = "last_host"
        private const val KEY_PORT = "last_port"
    }

    private lateinit var editHost: com.google.android.material.textfield.TextInputEditText
    private lateinit var editPort: com.google.android.material.textfield.TextInputEditText
    private lateinit var textStatus: TextView
    private lateinit var statusDot: View
    private lateinit var audioLevel: ProgressBar
    private lateinit var btnStartStop: MaterialButton
    private lateinit var textStats: TextView
    private lateinit var textPackets: TextView
    private lateinit var textReconnect: TextView
    private lateinit var btnDiscover: MaterialButton
    private lateinit var btnScanQr: MaterialButton
    private lateinit var textDiscovery: TextView

    private var service: AudioStreamService? = null
    private var bound = false
    private val handler = Handler(Looper.getMainLooper())
    private var discoveryManager: DiscoveryManager? = null
    private var discovering = false
    private var pendingAction: (() -> Unit)? = null

    private val qrScanLauncher = registerForActivityResult(
        ActivityResultContracts.StartActivityForResult()
    ) { result ->
        if (result.resultCode == Activity.RESULT_OK) {
            val host = result.data?.getStringExtra(QrScanActivity.EXTRA_HOST)
            val port = result.data?.getIntExtra(QrScanActivity.EXTRA_PORT, 0) ?: 0
            if (!host.isNullOrEmpty() && port > 0) {
                editHost.setText(host)
                editPort.setText(port.toString())
                Toast.makeText(this, "Scanned: $host:$port", Toast.LENGTH_SHORT).show()
            } else {
                Toast.makeText(this, "Invalid QR code", Toast.LENGTH_SHORT).show()
            }
        }
    }

    private val connection = object : ServiceConnection {
        override fun onServiceConnected(name: ComponentName?, binder: IBinder?) {
            val localBinder = binder as AudioStreamService.LocalBinder
            service = localBinder.getService()
            bound = true
            updateUi()
            startUiUpdates()
        }

        override fun onServiceDisconnected(name: ComponentName?) {
            service = null
            bound = false
        }
    }

    private val discoveryListener = object : DiscoveryManager.DiscoveryListener {
        override fun onServerFound(server: DiscoveryManager.DiscoveredServer) {
            handler.post {
                editHost.setText(server.host)
                editPort.setText(server.port.toString())
                textDiscovery.text = "\u2713 Found: ${server.name} (${server.host}:${server.port})"
                textDiscovery.setTextColor(ContextCompat.getColor(this@MainActivity, android.R.color.holo_green_dark))
                textDiscovery.visibility = View.VISIBLE
                stopDiscovery()
            }
        }

        override fun onServerLost(name: String) {
            handler.post {
                textDiscovery.visibility = View.GONE
            }
        }

        override fun onDiscoveryStarted() {
            handler.post {
                discovering = true
                btnDiscover.text = "Stop"
                textDiscovery.text = "\u23F3 Scanning for OpenDroidMic..."
                textDiscovery.setTextColor(
                    com.google.android.material.color.MaterialColors.getColor(
                        textDiscovery, com.google.android.material.R.attr.colorPrimary
                    )
                )
                textDiscovery.visibility = View.VISIBLE
            }
        }

        override fun onDiscoveryStopped() {
            handler.post {
                discovering = false
                btnDiscover.text = "Discover"
                if (textDiscovery.text?.startsWith("\u23F3") == true) {
                    textDiscovery.visibility = View.GONE
                }
            }
        }

        override fun onError(error: String) {
            handler.post {
                textDiscovery.text = "\u2717 $error"
                textDiscovery.setTextColor(ContextCompat.getColor(this@MainActivity, android.R.color.holo_red_dark))
                textDiscovery.visibility = View.VISIBLE
                discovering = false
                btnDiscover.text = "Discover"
            }
        }
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        setContentView(R.layout.activity_main)

        handleStopIntent(intent)

        editHost = findViewById(R.id.editHost)
        editPort = findViewById(R.id.editPort)
        textStatus = findViewById(R.id.textStatus)
        statusDot = findViewById(R.id.statusDot)
        audioLevel = findViewById(R.id.audioLevel)
        btnStartStop = findViewById(R.id.btnStartStop)
        textStats = findViewById(R.id.textStats)
        textPackets = findViewById(R.id.textPackets)
        textReconnect = findViewById(R.id.textReconnect)
        btnDiscover = findViewById(R.id.btnDiscover)
        btnScanQr = findViewById(R.id.btnScanQr)
        textDiscovery = findViewById(R.id.textDiscovery)

        discoveryManager = DiscoveryManager(this)

        val prefs = getSharedPreferences(PREFS_NAME, MODE_PRIVATE)
        val savedHost = prefs.getString(KEY_HOST, "")
        val savedPort = prefs.getInt(KEY_PORT, 0)
        if (!savedHost.isNullOrEmpty()) {
            editHost.setText(savedHost)
        }
        if (savedPort > 0) {
            editPort.setText(savedPort.toString())
        }

        btnStartStop.setOnClickListener {
            if (service?.isStreaming?.get() == true) {
                service?.stopStreaming()
            } else {
                startStreaming()
            }
        }

        btnDiscover.setOnClickListener {
            if (discovering) {
                stopDiscovery()
            } else {
                requestDiscoveryPermissionsAndStart()
            }
        }

        btnScanQr.setOnClickListener {
            requestCameraPermissionAndScan()
        }

        Intent(this, AudioStreamService::class.java).also { intent ->
            bindService(intent, connection, BIND_AUTO_CREATE)
        }
    }

    private fun requestCameraPermissionAndScan() {
        if (ContextCompat.checkSelfPermission(this, Manifest.permission.CAMERA)
            == PackageManager.PERMISSION_GRANTED
        ) {
            launchQrScanner()
        } else {
            pendingAction = { launchQrScanner() }
            ActivityCompat.requestPermissions(
                this,
                arrayOf(Manifest.permission.CAMERA),
                PERM_CAMERA
            )
        }
    }

    private fun launchQrScanner() {
        val intent = Intent(this, QrScanActivity::class.java)
        qrScanLauncher.launch(intent)
    }

    private fun requestDiscoveryPermissionsAndStart() {
        val permsNeeded = mutableListOf<String>()

        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
            if (ContextCompat.checkSelfPermission(this, Manifest.permission.NEARBY_WIFI_DEVICES)
                != PackageManager.PERMISSION_GRANTED
            ) {
                permsNeeded.add(Manifest.permission.NEARBY_WIFI_DEVICES)
            }
        } else {
            if (ContextCompat.checkSelfPermission(this, Manifest.permission.ACCESS_FINE_LOCATION)
                != PackageManager.PERMISSION_GRANTED
            ) {
                permsNeeded.add(Manifest.permission.ACCESS_FINE_LOCATION)
            }
        }

        if (permsNeeded.isEmpty()) {
            discoveryManager?.startDiscovery(discoveryListener)
        } else {
            pendingAction = { discoveryManager?.startDiscovery(discoveryListener) }
            ActivityCompat.requestPermissions(
                this,
                permsNeeded.toTypedArray(),
                PERM_DISCOVERY
            )
        }
    }

    private fun stopDiscovery() {
        discoveryManager?.stopDiscovery()
        discovering = false
        btnDiscover.text = "Discover"
    }

    private fun startStreaming() {
        val host = editHost.text?.toString()?.trim() ?: ""
        val port = editPort.text?.toString()?.trim()?.toIntOrNull() ?: 0

        if (host.isEmpty()) {
            editHost.error = "Enter an address"
            return
        }
        if (port <= 0 || port > 65535) {
            editPort.error = "Invalid port"
            return
        }

        if (ContextCompat.checkSelfPermission(this, Manifest.permission.RECORD_AUDIO)
            != PackageManager.PERMISSION_GRANTED
        ) {
            pendingAction = { startStreaming() }
            ActivityCompat.requestPermissions(
                this,
                arrayOf(Manifest.permission.RECORD_AUDIO),
                PERM_RECORD_AUDIO
            )
            return
        }

        service?.startStreaming(host, port)

        getSharedPreferences(PREFS_NAME, MODE_PRIVATE).edit()
            .putString(KEY_HOST, host)
            .putInt(KEY_PORT, port)
            .apply()

        updateUi()
    }

    override fun onRequestPermissionsResult(
        requestCode: Int, permissions: Array<out String>, grantResults: IntArray
    ) {
        super.onRequestPermissionsResult(requestCode, permissions, grantResults)
        val granted = grantResults.isNotEmpty() && grantResults[0] == PackageManager.PERMISSION_GRANTED

        when (requestCode) {
            PERM_RECORD_AUDIO -> {
                if (granted) {
                    pendingAction?.invoke()
                } else {
                    Toast.makeText(this, "Microphone permission is required to stream audio", Toast.LENGTH_LONG).show()
                }
            }
            PERM_CAMERA -> {
                if (granted) {
                    launchQrScanner()
                } else {
                    Toast.makeText(this, "Camera permission is required to scan QR codes", Toast.LENGTH_LONG).show()
                }
            }
            PERM_DISCOVERY -> {
                if (granted) {
                    discoveryManager?.startDiscovery(discoveryListener)
                } else {
                    val msg = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
                        "Nearby Devices permission is required to discover Linux PC"
                    } else {
                        "Location permission is required to discover devices on the network"
                    }
                    Toast.makeText(this, msg, Toast.LENGTH_LONG).show()
                }
            }
        }
        pendingAction = null
    }

    private fun startUiUpdates() {
        handler.postDelayed(object : Runnable {
            override fun run() {
                updateUi()
                if (bound) handler.postDelayed(this, UI_UPDATE_INTERVAL)
            }
        }, UI_UPDATE_INTERVAL)
    }

    private fun updateUi() {
        val svc = service
        if (svc == null) {
            textStatus.text = "Disconnected"
            statusDot.setBackgroundResource(R.drawable.status_dot_disconnected)
            btnStartStop.text = "Start Streaming"
            btnStartStop.setIconResource(android.R.drawable.ic_media_play)
            textStats.text = "48 kHz  \u2022  Mono  \u2022  Opus  \u2022  20ms frames"
            textPackets.text = ""
            textReconnect.visibility = TextView.GONE
            return
        }

        val streaming = svc.isStreaming.get()
        val state = svc.connectionState.get()

        val stateText: String
        val dotRes: Int
        val btnText: String
        val btnIcon: Int

        when (state) {
            AudioStreamService.State.DISCONNECTED -> {
                stateText = "Disconnected"
                dotRes = R.drawable.status_dot_disconnected
                btnText = "Start Streaming"
                btnIcon = android.R.drawable.ic_media_play
            }
            AudioStreamService.State.CONNECTING -> {
                stateText = "Connecting..."
                dotRes = R.drawable.status_dot_connecting
                btnText = "Connecting..."
                btnIcon = android.R.drawable.ic_delete
            }
            AudioStreamService.State.WAITING_ACK -> {
                stateText = "Waiting for server..."
                dotRes = R.drawable.status_dot_connecting
                btnText = "Cancel"
                btnIcon = android.R.drawable.ic_delete
            }
            AudioStreamService.State.CONNECTED -> {
                stateText = "Connected"
                dotRes = R.drawable.status_dot_connected
                btnText = "Stop Streaming"
                btnIcon = android.R.drawable.ic_media_pause
            }
            AudioStreamService.State.STREAMING -> {
                stateText = "Streaming"
                dotRes = R.drawable.status_dot_connected
                btnText = "Stop Streaming"
                btnIcon = android.R.drawable.ic_media_pause
            }
            AudioStreamService.State.RECONNECTING -> {
                stateText = "Reconnecting..."
                dotRes = R.drawable.status_dot_connecting
                btnText = "Cancel"
                btnIcon = android.R.drawable.ic_delete
            }
            AudioStreamService.State.ERROR -> {
                stateText = "Connection failed"
                dotRes = R.drawable.status_dot_error
                btnText = "Retry"
                btnIcon = android.R.drawable.ic_media_play
            }
            else -> {
                stateText = "Disconnected"
                dotRes = R.drawable.status_dot_disconnected
                btnText = "Start Streaming"
                btnIcon = android.R.drawable.ic_media_play
            }
        }

        textStatus.text = stateText
        statusDot.setBackgroundResource(dotRes)
        btnStartStop.text = btnText
        btnStartStop.setIconResource(btnIcon)
        audioLevel.progress = svc.currentAudioLevel.get()

        val sent = svc.packetsSent.get()
        val lost = svc.packetsLost.get()
        textStats.text = "48 kHz  \u2022  Mono  \u2022  Opus  \u2022  20ms frames"
        textPackets.text = if (sent > 0 || lost > 0) "$sent sent  \u2022  $lost lost" else ""

        val reconnectAttempt = svc.reconnectAttempts.get()
        if (state == AudioStreamService.State.RECONNECTING && reconnectAttempt > 0) {
            textReconnect.visibility = TextView.VISIBLE
            textReconnect.text = "Attempt $reconnectAttempt / ${AudioStreamService.MAX_RECONNECT_ATTEMPTS}"
        } else if (state == AudioStreamService.State.ERROR) {
            textReconnect.visibility = TextView.VISIBLE
            textReconnect.text = "Check that the Linux receiver is running on the correct port"
        } else {
            textReconnect.visibility = TextView.GONE
        }
    }

    override fun onDestroy() {
        super.onDestroy()
        stopDiscovery()
        if (bound) {
            unbindService(connection)
            bound = false
        }
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        handleStopIntent(intent)
    }

    private fun handleStopIntent(intent: Intent?) {
        if (intent?.action == "ACTION_STOP") {
            if (bound) {
                service?.stopStreaming()
            } else {
                pendingAction = { service?.stopStreaming() }
                Intent(this, AudioStreamService::class.java).also { svcIntent ->
                    bindService(svcIntent, object : ServiceConnection {
                        override fun onServiceConnected(name: ComponentName?, binder: IBinder?) {
                            val svc = (binder as AudioStreamService.LocalBinder).getService()
                            svc.stopStreaming()
                            unbindService(this)
                        }
                        override fun onServiceDisconnected(name: ComponentName?) {}
                    }, BIND_AUTO_CREATE)
                }
            }
        }
    }
}
