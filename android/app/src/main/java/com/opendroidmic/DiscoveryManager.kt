package com.opendroidmic

import android.content.Context
import android.net.nsd.NsdManager
import android.net.nsd.NsdServiceInfo
import android.os.Build
import android.util.Log

class DiscoveryManager(context: Context) {
    companion object {
        private const val TAG = "DiscoveryManager"
        private const val SERVICE_TYPE = "_opendroidmic._udp."
    }

    data class DiscoveredServer(
        val name: String,
        val host: String,
        val port: Int,
    )

    interface DiscoveryListener {
        fun onServerFound(server: DiscoveredServer)
        fun onServerLost(name: String)
        fun onDiscoveryStarted()
        fun onDiscoveryStopped()
        fun onError(error: String)
    }

    private val nsdManager = context.getSystemService(Context.NSD_SERVICE) as NsdManager
    private var discoveryListener: NsdManager.DiscoveryListener? = null
    private var resolvingService: NsdServiceInfo? = null

    fun startDiscovery(listener: DiscoveryListener) {
        stopDiscovery()

        val resolveListener = object : NsdManager.ResolveListener {
            override fun onResolveFailed(serviceInfo: NsdServiceInfo, errorCode: Int) {
                Log.w(TAG, "Resolve failed for ${serviceInfo.serviceName}: error $errorCode")
            }

            override fun onServiceResolved(serviceInfo: NsdServiceInfo) {
                val host = serviceInfo.host?.hostAddress
                if (host == null) {
                    Log.w(TAG, "Resolved but host is null for ${serviceInfo.serviceName}")
                    return
                }
                if (host.contains(":")) {
                    Log.d(TAG, "Ignoring IPv6 address $host for ${serviceInfo.serviceName}")
                    return
                }
                val port = serviceInfo.port
                val name = serviceInfo.serviceName

                Log.d(TAG, "Resolved: $name at $host:$port")
                listener.onServerFound(DiscoveredServer(name, host, port))
            }
        }

        discoveryListener = object : NsdManager.DiscoveryListener {
            override fun onDiscoveryStarted(regType: String) {
                Log.d(TAG, "Discovery started for $regType")
                listener.onDiscoveryStarted()
            }

            override fun onServiceFound(service: NsdServiceInfo) {
                Log.d(TAG, "Service found: ${service.serviceName} type=${service.serviceType}")

                val serviceType = service.serviceType ?: ""
                if (serviceType.startsWith("_opendroidmic")) {
                    resolvingService = service
                    try {
                        nsdManager.resolveService(service, resolveListener)
                    } catch (e: Exception) {
                        Log.e(TAG, "resolveService failed", e)
                    }
                }
            }

            override fun onServiceLost(service: NsdServiceInfo) {
                Log.d(TAG, "Service lost: ${service.serviceName}")
                listener.onServerLost(service.serviceName)
            }

            override fun onDiscoveryStopped(serviceType: String) {
                Log.d(TAG, "Discovery stopped")
                listener.onDiscoveryStopped()
            }

            override fun onStartDiscoveryFailed(serviceType: String, errorCode: Int) {
                Log.e(TAG, "Discovery start failed: errorCode=$errorCode")
                listener.onError("mDNS discovery failed (error $errorCode). Ensure Wi-Fi is enabled.")
                try {
                    nsdManager.stopServiceDiscovery(this)
                } catch (_: Exception) {}
            }

            override fun onStopDiscoveryFailed(serviceType: String, errorCode: Int) {
                Log.e(TAG, "Discovery stop failed: $errorCode")
            }
        }

        try {
            nsdManager.discoverServices(
                SERVICE_TYPE,
                NsdManager.PROTOCOL_DNS_SD,
                discoveryListener
            )
        } catch (e: Exception) {
            Log.e(TAG, "Failed to start discovery", e)
            listener.onError("Failed to start discovery: ${e.message}")
        }
    }

    fun stopDiscovery() {
        try {
            discoveryListener?.let {
                nsdManager.stopServiceDiscovery(it)
            }
        } catch (_: Exception) {
        }
        discoveryListener = null
        resolvingService = null
    }
}
