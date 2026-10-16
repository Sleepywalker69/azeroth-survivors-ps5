/*
 * Azeroth Survivors for the PlayStation 5 - the title's main.
 *
 * PS5_Vulkan's CRT (tooling/native/app_crt.cpp) runs the constructors, calls
 * this main and hands its status to catchReturnFromMain, which asks the shell
 * to close the title. The game itself is Rust (crates/survivors-ps5, linked in
 * as a static library): this sends standard error to klog, dismisses the
 * splash, points the game at its data and its state folder, and runs it on a
 * thread with a stack the size a desktop main thread has.
 *
 * A title sees its own folder (/app0, which is /data/homebrew/<TITLE_ID>):
 *
 *   /app0/Data/       the game's MPQ archives (patch.MPQ, patch-2.MPQ, ...)
 *   /app0/userdata/   settings, the session, logs, crash reports
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

int survivors_ps5_main(void);
void catchReturnFromMain(int status);

/* The game may leave through exit() (Rust's process::exit, an abort's handler):
 * the shell is asked to close the title then too, or it shows it as crashed. */
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

static int game_status;

static void *
game(void *unused)
{
   (void)unused;
   game_status = survivors_ps5_main();
   return NULL;
}

int
main(int argc, char **argv)
{
   (void)argc;
   (void)argv;
   platform_init("AzerothSurvivors");
   /* The game prints its banners and executor notes to standard output: klog too. */
   dup2(STDERR_FILENO, STDOUT_FILENO);
   atexit(leave);

   mkdir("/app0/userdata", 0777);
   read_env_file("/app0/env.txt");
   /* Where benilla looks for the install and keeps its state; env.txt may move either. */
   setenv("WOW_DATA", "/app0/Data", 0);
   setenv("BENILLA_HOME", "/app0/userdata", 0);
   setenv("HOME", "/app0/userdata", 0);
   setenv("TMPDIR", "/app0/userdata", 0);

   struct stat data;
   if (stat(getenv("WOW_DATA"), &data) != 0)
      say("no game data at %s: copy the Data folder (patch.MPQ, ...) there", getenv("WOW_DATA"));

   pthread_attr_t attributes;
   pthread_attr_init(&attributes);
   pthread_attr_setstacksize(&attributes, 64u << 20);
   pthread_t thread;
   if (pthread_create(&thread, &attributes, game, NULL) != 0) {
      say("could not start the game's thread; running it on main's");
      game(NULL);
   } else {
      pthread_join(thread, NULL);
   }
   pthread_attr_destroy(&attributes);
   say("the game returned %d", game_status);
   return game_status;
}
