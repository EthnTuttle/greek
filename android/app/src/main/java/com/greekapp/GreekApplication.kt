package com.greekapp

import android.app.Application
import android.util.Log

class GreekApplication : Application() {
    override fun onCreate() {
        super.onCreate()

        Log.d("GreekApp", "Greek app initialized")

        // TODO: Later, when Rust library is built via UniFFI:
        // System.loadLibrary("uniffi_bindings")
        // GreekCore.initialize(applicationContext.getDatabasePath("morphology.sqlite").absolutePath)
    }
}
