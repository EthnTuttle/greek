package com.greekapp.navigation

import android.app.Activity
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Book
import androidx.compose.material.icons.filled.Home
import androidx.compose.material.icons.filled.MenuBook
import androidx.compose.material.icons.filled.Quiz
import androidx.compose.material.icons.filled.Search
import androidx.compose.material.icons.filled.Settings
import androidx.compose.material.icons.outlined.Book
import androidx.compose.material.icons.outlined.Home
import androidx.compose.material.icons.outlined.MenuBook
import androidx.compose.material.icons.outlined.Quiz
import androidx.compose.material.icons.outlined.Search
import androidx.compose.material.icons.outlined.Settings
import androidx.compose.material3.*
import androidx.compose.material3.windowsizeclass.ExperimentalMaterial3WindowSizeClassApi
import androidx.compose.material3.windowsizeclass.WindowWidthSizeClass
import androidx.compose.material3.windowsizeclass.calculateWindowSizeClass
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.platform.LocalContext
import androidx.navigation.NavController
import androidx.navigation.NavDestination
import androidx.navigation.NavDestination.Companion.hierarchy
import androidx.navigation.NavGraph.Companion.findStartDestination
import androidx.navigation.NavType
import androidx.navigation.compose.NavHost
import androidx.navigation.compose.composable
import androidx.navigation.compose.currentBackStackEntryAsState
import androidx.navigation.navArgument
import androidx.navigation.compose.rememberNavController
import com.greekapp.ui.screens.*
import com.greekapp.viewmodel.PracticeViewModel

sealed class Screen(
    val route: String,
    val title: String,
    val selectedIcon: ImageVector,
    val unselectedIcon: ImageVector
) {
    object Home : Screen("home", "Home", Icons.Filled.Home, Icons.Outlined.Home)
    object Review : Screen("review", "Review", Icons.Filled.MenuBook, Icons.Outlined.MenuBook)
    // Quiz comes from material-icons-extended (declared in app/build.gradle.kts).
    object Practice : Screen("practice", "Practice", Icons.Filled.Quiz, Icons.Outlined.Quiz)
    // Wave-5 Alphabet Check: pushed destination, NOT a bottom-nav tab (like Review).
    object AlphabetCheck : Screen("alphabet-check", "Alphabet Check", Icons.Filled.Quiz, Icons.Outlined.Quiz)
    object Reader : Screen("reader", "Read", Icons.Filled.Book, Icons.Outlined.Book)
    object Curriculum : Screen("curriculum", "Progress", Icons.Filled.MenuBook, Icons.Outlined.MenuBook)
    object Dictionary : Screen("dictionary", "Dictionary", Icons.Filled.Search, Icons.Outlined.Search)
    object Settings : Screen("settings", "Settings", Icons.Filled.Settings, Icons.Outlined.Settings)
    // Wave-6 Device Sync: pushed destination, NOT a bottom-nav tab (like Review).
    object Sync : Screen("sync", "Sync", Icons.Filled.Settings, Icons.Outlined.Settings)
}

val bottomNavItems = listOf(
    Screen.Home,
    Screen.Practice,
    Screen.Reader,
    Screen.Dictionary,
    Screen.Curriculum,
    Screen.Settings
)

private fun NavController.navigateToTab(route: String) {
    navigate(route) {
        // Pop up to the start destination of the graph to
        // avoid building up a large stack of destinations
        popUpTo(graph.findStartDestination().id) {
            saveState = true
        }
        // Avoid multiple copies of the same destination when
        // reselecting the same item
        launchSingleTop = true
        // Restore state when reselecting a previously selected item
        restoreState = true
    }
}

@OptIn(ExperimentalMaterial3Api::class, ExperimentalMaterial3WindowSizeClassApi::class)
@Composable
fun AppNavigation() {
    val navController = rememberNavController()
    val navBackStackEntry by navController.currentBackStackEntryAsState()
    val currentDestination = navBackStackEntry?.destination

    // Use a side navigation rail on medium+ widths (tablets / unfolded foldables /
    // landscape), and the bottom navigation bar on compact (phone) widths.
    val activity = LocalContext.current as Activity
    val windowSizeClass = calculateWindowSizeClass(activity)
    val useNavRail = windowSizeClass.widthSizeClass != WindowWidthSizeClass.Compact

    if (useNavRail) {
        Row(modifier = Modifier.fillMaxSize()) {
            NavigationRail {
                bottomNavItems.forEach { screen ->
                    val selected = currentDestination.isSelected(screen)
                    NavigationRailItem(
                        icon = {
                            Icon(
                                imageVector = if (selected) screen.selectedIcon else screen.unselectedIcon,
                                contentDescription = screen.title
                            )
                        },
                        label = { Text(screen.title) },
                        selected = selected,
                        onClick = { navController.navigateToTab(screen.route) }
                    )
                }
            }
            AppNavHost(navController = navController, modifier = Modifier.fillMaxSize())
        }
    } else {
        Scaffold(
            bottomBar = {
                NavigationBar {
                    bottomNavItems.forEach { screen ->
                        val selected = currentDestination.isSelected(screen)
                        NavigationBarItem(
                            icon = {
                                Icon(
                                    imageVector = if (selected) screen.selectedIcon else screen.unselectedIcon,
                                    contentDescription = screen.title
                                )
                            },
                            label = { Text(screen.title) },
                            selected = selected,
                            onClick = { navController.navigateToTab(screen.route) }
                        )
                    }
                }
            }
        ) { innerPadding ->
            AppNavHost(navController = navController, modifier = Modifier.padding(innerPadding))
        }
    }
}

private fun NavDestination?.isSelected(screen: Screen): Boolean =
    this?.hierarchy?.any { it.route == screen.route } == true

@Composable
private fun AppNavHost(navController: androidx.navigation.NavHostController, modifier: Modifier = Modifier) {
    NavHost(
        navController = navController,
        startDestination = Screen.Home.route,
        modifier = modifier
    ) {
        composable(Screen.Home.route) {
            HomeScreen(
                onStartReview = {
                    navController.navigate(Screen.Review.route)
                },
                onContinueReading = {
                    navController.navigate(Screen.Reader.route)
                },
                onOpenLetters = {
                    // Curriculum is a bottom-nav tab: reuse the tab-navigation
                    // helper so state save/restore matches the nav bar behavior.
                    navController.navigateToTab(Screen.Curriculum.route)
                },
                onTakeAlphabetCheck = {
                    // Stacked on top of Home like Review; nav-pop returns here.
                    navController.navigate(Screen.AlphabetCheck.route)
                }
            )
        }

        composable(Screen.Review.route) {
            ReviewScreen(
                onComplete = {
                    navController.popBackStack()
                }
            )
        }

        composable(Screen.Practice.route) {
            PracticeScreen(
                onBack = {
                    navController.popBackStack()
                },
                onStudyLetters = {
                    navController.navigateToTab(Screen.Curriculum.route)
                }
            )
        }

        // Wave-5 Alphabet Check destination. Separate route (no query-arg
        // support exists in this NavHost); the constant-default nav argument
        // is what flags check mode to the shared PracticeViewModel through
        // its SavedStateHandle — the screen itself needs no mode parameter.
        composable(
            route = Screen.AlphabetCheck.route,
            arguments = listOf(
                navArgument(PracticeViewModel.ARG_ALPHABET_CHECK) {
                    type = NavType.BoolType
                    defaultValue = true
                }
            )
        ) {
            PracticeScreen(
                onBack = {
                    navController.popBackStack()
                },
                onStudyLetters = {
                    navController.navigateToTab(Screen.Curriculum.route)
                }
            )
        }

        composable(Screen.Reader.route) {
            ReaderScreen()
        }

        composable(Screen.Curriculum.route) {
            CurriculumScreen(
                onTakeAlphabetCheck = {
                    navController.navigate(Screen.AlphabetCheck.route)
                }
            )
        }

        composable(Screen.Dictionary.route) {
            DictionaryScreen()
        }

        composable(Screen.Settings.route) {
            SettingsScreen(
                onTakeAlphabetCheck = {
                    navController.navigate(Screen.AlphabetCheck.route)
                },
                onOpenSync = {
                    navController.navigate(Screen.Sync.route)
                }
            )
        }

        // Wave-6 Device Sync destination (pushed from Settings).
        composable(Screen.Sync.route) {
            SyncScreen(
                onBack = {
                    navController.popBackStack()
                }
            )
        }
    }
}
