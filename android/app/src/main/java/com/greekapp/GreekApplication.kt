package com.greekapp

import android.app.Application
import android.util.Log
import org.json.JSONArray
import uniffi.uniffi_bindings.GreekCore
import java.io.File

class GreekApplication : Application() {

    companion object {
        var greekCore: GreekCore? = null
            private set
        var isInitialized: Boolean = false
            private set
    }

    override fun onCreate() {
        super.onCreate()
        System.loadLibrary("uniffi_bindings")

        try {
            // Copy morphology.sqlite from assets to internal storage if not already present
            val morphFile = getDatabasePath("morphology.sqlite")
            if (!morphFile.exists()) {
                morphFile.parentFile?.mkdirs()
                assets.open("morphology.sqlite").use { input ->
                    morphFile.outputStream().use { output ->
                        input.copyTo(output)
                    }
                }
            }

            // Read all card JSON files from assets/cards/ and merge into one JSON array
            val cardFiles = listOf("athenaze-i.json", "athenaze-ii.json", "jact.json", "dcc-core.json")
            val allCards = JSONArray()
            for (fileName in cardFiles) {
                val json = assets.open("cards/$fileName").bufferedReader().use { it.readText() }
                val arr = JSONArray(json)
                for (i in 0 until arr.length()) {
                    allCards.put(arr.get(i))
                }
            }
            val allCardsJson = allCards.toString()

            // Create the main app database path
            val dbPath = getDatabasePath("greek_app.db").absolutePath

            // Initialize the Rust core via UniFFI
            greekCore = GreekCore.newWithCards(dbPath, morphFile.absolutePath, allCardsJson)
            isInitialized = true

            Log.d("GreekApp", "GreekCore initialized successfully")
        } catch (e: Exception) {
            Log.e("GreekApp", "Failed to initialize GreekCore", e)
        }
    }
}
