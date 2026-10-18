import java.util.Properties

plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.android")
    id("org.jetbrains.kotlin.plugin.compose")
}

android {
    namespace = "com.greekapp"
    compileSdk = 35

    // Release signing (zapstore/GitHub distribution): the keystore lives
    // outside git (see android/keystore/ + keystore.properties, both
    // gitignored). `gradlew assembleRelease` signs when the properties file
    // exists on this machine, and produces an unsigned APK otherwise.
    val keystoreProperties = Properties().apply {
        val f = rootProject.file("keystore.properties")
        if (f.exists()) f.inputStream().use { load(it) }
    }
    val hasReleaseKey = keystoreProperties.getProperty("storeFile") != null

    defaultConfig {
        applicationId = "com.greekapp"
        minSdk = 26
        targetSdk = 35
        versionCode = 3
        versionName = "0.1.1"

        testInstrumentationRunner = "androidx.test.runner.AndroidJUnitRunner"
        vectorDrawables {
            useSupportLibrary = true
        }
    }

    signingConfigs {
        create("release") {
            storeFile = keystoreProperties.getProperty("storeFile")?.let { rootProject.file(it) }
            storePassword = keystoreProperties.getProperty("storePassword")
            keyAlias = keystoreProperties.getProperty("keyAlias")
            keyPassword = keystoreProperties.getProperty("keyPassword")
        }
    }

    buildTypes {
        release {
            if (hasReleaseKey) signingConfig = signingConfigs.getByName("release")
            isMinifyEnabled = false
            proguardFiles(
                getDefaultProguardFile("proguard-android-optimize.txt"),
                "proguard-rules.pro"
            )
        }
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    kotlinOptions {
        jvmTarget = "17"
    }

    buildFeatures {
        compose = true
    }

    packaging {
        resources {
            excludes += "/META-INF/{AL2.0,LGPL2.1}"
        }
    }
}

dependencies {
    // Compose BOM
    val composeBom = platform("androidx.compose:compose-bom:2024.12.01")
    implementation(composeBom)
    androidTestImplementation(composeBom)

    // Core Android
    implementation("androidx.core:core-ktx:1.15.0")
    implementation("androidx.lifecycle:lifecycle-runtime-ktx:2.8.7")
    implementation("androidx.activity:activity-compose:1.9.3")

    // Compose
    implementation("androidx.compose.ui:ui")
    implementation("androidx.compose.ui:ui-graphics")
    implementation("androidx.compose.ui:ui-tooling-preview")
    implementation("androidx.compose.material3:material3")
    implementation("androidx.compose.material3:material3-window-size-class")
    implementation("androidx.compose.material:material-icons-extended")

    // Navigation
    implementation("androidx.navigation:navigation-compose:2.8.5")

    // DataStore (persisted settings)
    implementation("androidx.datastore:datastore-preferences:1.1.1")

    // ViewModel
    implementation("androidx.lifecycle:lifecycle-viewmodel-compose:2.8.7")
    implementation("androidx.lifecycle:lifecycle-runtime-compose:2.8.7")

    // JNA for UniFFI bindings
    implementation("net.java.dev.jna:jna:5.14.0@aar")

    // Wave-6 device sync: QR rendering for iroh pairing tickets
    implementation("com.google.zxing:core:3.5.3")

    // Debug
    debugImplementation("androidx.compose.ui:ui-tooling")
    debugImplementation("androidx.compose.ui:ui-test-manifest")

    // Testing
    testImplementation("junit:junit:4.13.2")
    androidTestImplementation("androidx.test.ext:junit:1.2.1")
    androidTestImplementation("androidx.test.espresso:espresso-core:3.6.1")
    androidTestImplementation("androidx.compose.ui:ui-test-junit4")
}

// Syncs the shared card JSON + morphology DB from the repo's data/ directory into
// the app's assets so contributors don't have to copy files by hand. Idempotent:
// Copy is up-to-date-aware and only re-copies when sources change.
val dataDir = rootProject.projectDir.parentFile.resolve("data")
val assetsDir = projectDir.resolve("src/main/assets")

val syncCardData by tasks.registering(Copy::class) {
    description = "Copy card JSON files from data/cards into app assets."
    group = "greek"
    from(dataDir.resolve("cards")) {
        include("*.json")
    }
    into(assetsDir.resolve("cards"))
}

val syncMorphologyDb by tasks.registering(Copy::class) {
    description = "Copy the morphology SQLite database from data/ into app assets."
    group = "greek"
    from(dataDir) {
        include("morphology.sqlite")
    }
    into(assetsDir)
}

val syncAssets by tasks.registering {
    description = "Sync all shared data files (cards + morphology DB) into app assets."
    group = "greek"
    dependsOn(syncCardData, syncMorphologyDb)
}

tasks.named("preBuild") {
    dependsOn(syncAssets)
}
