/*
 * Azeroth Survivors (Aethus) for the PlayStation 5 - the title's main.
 *
 * azeroth-survivors-re's ps5/src/main.c with the Aethus host in place of the game:
 * PS5_Vulkan's CRT runs the constructors, calls this main and hands its status to
 * catchReturnFromMain, which asks the shell to close the title. This sends standard error to
 * klog, dismisses the splash, sets the environment the host reads and runs it on a thread
 * with a stack the size a desktop main thread has (the translated wasm recurses on it).
 *
 * A title sees its own folder (/app0, which is /data/homebrew/<TITLE_ID>):
 *
 *   /app0/js/         the host's scripts (env.js, gpu.js, audio.js, boot.mjs)
 *   /app0/web/        the web build (wenilla.js, snippets/)
 *   /app0/Data/       the MPQs (patch.MPQ, patch-2.MPQ ... patch-5.MPQ)
 *   /app0/userdata/   settings, saves, AddOns, the log
 *   /app0/env.txt     optional: NAME=value lines for the environment
 *
 * SPDX-License-Identifier: MIT
 */
#include <pthread.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>
#include <unistd.h>

#include "platform.h"

int aethus_ps5_main(void);
void catchReturnFromMain(int status);

/* The host may leave through exit() (an abort's handler): the shell is asked to close the
 * title then too, or it shows it as crashed. */
static void
leave(void)
{
   static int left;

   if (!left++)
      catchReturnFromMain(0);
}

static void
read_env_file(const char *path)
{
   FILE *file = fopen(path, "r");
   if (!file)
      return;
   char line[512];
   while (fgets(line, sizeof line, file)) {
      line[strcspn(line, "\r\n")] = 0;
      char *equals = strchr(line, '=');
      if (line[0] == '#' || !equals || equals == line)
         continue;
      *equals = 0;
      setenv(line, equals + 1, 1);
      say("env.txt: %s=%s", line, equals + 1);
   }
   fclose(file);
}

static int host_status;

static void *
host(void *unused)
{
   (void)unused;
   host_status = aethus_ps5_main();
   return NULL;
}

int
main(int argc, char **argv)
{
   (void)argc;
   (void)argv;
   platform_init("AzerothAethus");
   dup2(STDERR_FILENO, STDOUT_FILENO);
   atexit(leave);

   mkdir("/app0/userdata", 0777);
   read_env_file("/app0/env.txt");
   setenv("WOW_DATA", "/app0/Data", 0);
   setenv("BENILLA_HOME", "/app0/userdata", 0);
   setenv("HOME", "/app0/userdata", 0);
   setenv("TMPDIR", "/app0/userdata", 0);
   /* The display surface's size (the patched wgpu-hal reads it); VideoOut scales it to the
    * screen. 1080p by default: the game runs single-threaded, as in a browser.
    * env.txt may ask for 3840x2160 or 2560x1440. */
   setenv("PS5_DISPLAY_MODE", "1920x1080", 0);

   struct stat data;
   if (stat("/app0/Data", &data) != 0 && !getenv("WOW_DATA"))
      say("no game data at /app0/Data: copy the MPQs there");

   pthread_attr_t attributes;
   pthread_attr_init(&attributes);
   pthread_attr_setstacksize(&attributes, 256u << 20);
   pthread_t thread;
   if (pthread_create(&thread, &attributes, host, NULL) != 0) {
      say("could not start the host's thread; running it on main's");
      host(NULL);
   } else {
      pthread_join(thread, NULL);
   }
   pthread_attr_destroy(&attributes);
   say("the host returned %d", host_status);
   return host_status;
}
