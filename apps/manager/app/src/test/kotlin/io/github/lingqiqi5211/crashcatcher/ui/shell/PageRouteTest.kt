package io.github.lingqiqi5211.crashcatcher.ui.shell

import org.junit.Assert.assertEquals
import org.junit.Test

class PageRouteTest {

    @Test
    fun `an app route preserves its Android user`() {
        val page = Page.AppDetail("com.example", userId = 10)

        assertEquals(page, page.toRoute().toPage())
    }

    @Test
    fun `an old app route restores into the owner user`() {
        assertEquals(Page.AppDetail("com.example", userId = 0), "app/com.example".toPage())
    }

    @Test
    fun `a platform process path survives the user route`() {
        val page = Page.AppDetail("/vendor/bin/hw/example", userId = 10)

        assertEquals(page, page.toRoute().toPage())
    }
}
