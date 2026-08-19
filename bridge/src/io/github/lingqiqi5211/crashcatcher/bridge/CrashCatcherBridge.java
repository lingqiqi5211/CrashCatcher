package io.github.lingqiqi5211.crashcatcher.bridge;

import android.app.Notification;
import android.app.NotificationChannel;
import android.app.NotificationManager;
import android.app.PendingIntent;
import android.content.Context;
import android.content.Intent;
import android.content.pm.ApplicationInfo;
import android.content.pm.PackageInfo;
import android.content.pm.PackageManager;
import android.graphics.drawable.Icon;
import android.net.LocalSocket;
import android.net.LocalSocketAddress;
import android.os.Build;
import android.os.Looper;
import android.os.Process;
import android.os.UserHandle;
import android.system.Os;
import android.util.Log;

import org.json.JSONArray;
import org.json.JSONException;
import org.json.JSONObject;

import java.io.DataInputStream;
import java.io.DataOutputStream;
import java.io.EOFException;
import java.io.IOException;
import java.lang.reflect.Method;
import java.nio.charset.StandardCharsets;

public final class CrashCatcherBridge {
    private static final String TAG = "CrashCatcherBridge";
    private static final String VERSION = "0.1.0";
    private static final String CHANNEL_ID = "crashcatcher_crashes";
    private static final int MAX_FRAME_BYTES = 1024 * 1024;
    private static final int SYSTEM_UID = 1000;

    private final Context context;
    private final String socketName;
    private final String managerPackage;

    private CrashCatcherBridge(Context context, String socketName, String managerPackage) {
        this.context = context;
        this.socketName = socketName;
        this.managerPackage = managerPackage;
    }

    public static void main(String[] args) {
        try {
            Arguments parsed = Arguments.parse(args);
            if (Looper.getMainLooper() == null) {
                Looper.prepareMainLooper();
            }
            Context context = systemContext();
            dropToSystemUid();
            new CrashCatcherBridge(context, parsed.socketName, parsed.managerPackage).runForever();
        } catch (Throwable error) {
            Log.e(TAG, "bridge initialization failed", error);
            throw new IllegalStateException("bridge initialization failed", error);
        }
    }

    private void runForever() {
        while (true) {
            try {
                runConnection();
            } catch (Throwable error) {
                Log.e(TAG, "bridge connection failed; reconnecting", error);
                try {
                    Thread.sleep(1000L);
                } catch (InterruptedException interrupted) {
                    Thread.currentThread().interrupt();
                    return;
                }
            }
        }
    }

    private void runConnection() throws IOException, JSONException {
        try (LocalSocket socket = new LocalSocket()) {
            socket.connect(new LocalSocketAddress(socketName, LocalSocketAddress.Namespace.ABSTRACT));
            DataInputStream input = new DataInputStream(socket.getInputStream());
            DataOutputStream output = new DataOutputStream(socket.getOutputStream());
            writeFrame(output, new JSONObject()
                    .put("type", "hello")
                    .put("hello", new JSONObject()
                            .put("android_sdk", Build.VERSION.SDK_INT)
                            .put("bridge_version", VERSION)));
            while (true) {
                JSONObject command = readFrame(input);
                handle(command, output);
            }
        } catch (EOFException disconnected) {
            Log.w(TAG, "daemon disconnected");
        }
    }

    private void handle(JSONObject command, DataOutputStream output) throws IOException, JSONException {
        long requestId = command.optLong("request_id", 0L);
        String type = command.optString("type", "");
        try {
            switch (type) {
                case "post_notification":
                    postNotification(command.getJSONObject("notification"));
                    writeFrame(output, result("notification_result", requestId, "posted", true, null));
                    break;
                case "cancel_notification":
                    cancelNotification(command.getString("record_id"));
                    writeFrame(output, result("notification_result", requestId, "posted", true, null));
                    break;
                case "query_package_info":
                    JSONObject packageInfo = queryPackageInfo(
                            command.getString("package_name"), command.getInt("user_id"));
                    writeFrame(output, new JSONObject()
                            .put("type", "package_info_result")
                            .put("request_id", requestId)
                            .put("package", packageInfo)
                            .put("error", JSONObject.NULL));
                    break;
                case "start_activity":
                    startActivity(command.getJSONObject("intent"), command.getInt("user_id"));
                    writeFrame(output, result("activity_result", requestId, "launched", true, null));
                    break;
                default:
                    throw new IllegalArgumentException("unknown bridge command: " + type);
            }
        } catch (Throwable error) {
            String resultType = type.contains("notification")
                    ? "notification_result"
                    : type.equals("query_package_info") ? "package_info_result" : "activity_result";
            String booleanField = resultType.equals("notification_result") ? "posted" : "launched";
            writeFrame(output, result(resultType, requestId, booleanField, false, error.toString()));
        }
    }

    private void postNotification(JSONObject spec) throws JSONException {
        NotificationManager manager = context.getSystemService(NotificationManager.class);
        if (manager == null) {
            throw new IllegalStateException("NotificationManager unavailable");
        }
        if (Build.VERSION.SDK_INT >= 26) {
            // Channel name and description are what the user sees in system settings, so
            // they follow the app's language rather than staying English.
            NotificationChannel channel = new NotificationChannel(
                    CHANNEL_ID, "应用崩溃", NotificationManager.IMPORTANCE_HIGH);
            channel.setDescription("崩溃捕手的崩溃与 ANR 提醒");
            manager.createNotificationChannel(channel);
        }
        String recordId = spec.getString("record_id");
        int userId = spec.getInt("user_id");
        Notification.Builder builder = Build.VERSION.SDK_INT >= 26
                ? new Notification.Builder(context, CHANNEL_ID)
                : new Notification.Builder(context);
        builder.setSmallIcon(android.R.drawable.stat_notify_error)
                .setContentTitle(notificationTitle(spec, userId))
                .setContentText(spec.getString("body"))
                .setStyle(new Notification.BigTextStyle().bigText(spec.getString("body")))
                .setAutoCancel(true)
                .setCategory(Notification.CATEGORY_ERROR)
                .setContentIntent(managerIntent(recordId, "open_details", userId, 0));

        JSONArray actions = spec.optJSONArray("actions");
        if (actions != null) {
            for (int index = 0; index < actions.length(); index++) {
                JSONObject action = actions.getJSONObject(index);
                PendingIntent intent = managerIntent(
                        recordId, action.getString("action"), userId, index + 1);
                builder.addAction(new Notification.Action.Builder(
                        Icon.createWithResource(context, android.R.drawable.ic_menu_view),
                        action.getString("title"), intent).build());
            }
        }
        manager.notify(recordId, recordId.hashCode(), builder.build());
    }

    /**
     * The notification title, with the crashing app's display name where it is known.
     *
     * The daemon can only supply the package name — it has no access to Android
     * resources — but this process is privileged and sees every package, so it swaps in
     * the launcher label. `io.github.example.app 已崩溃` is not how a user knows which app
     * just died. Falls back to whatever the daemon sent if the package has since been
     * uninstalled, which is a normal outcome for a crash recorded moments earlier.
     */
    private String notificationTitle(JSONObject spec, int userId) {
        String fallback = spec.optString("title");
        String packageName = spec.optString("package_name");
        if (packageName == null || packageName.isEmpty()) {
            return fallback;
        }
        try {
            Context userContext = contextAsUser(context, userId);
            PackageManager packageManager = userContext.getPackageManager();
            CharSequence label = packageManager.getApplicationLabel(
                    packageManager.getApplicationInfo(packageName, 0));
            if (label != null && label.length() > 0) {
                return label + " 已崩溃";
            }
        } catch (Exception ignored) {
            // Uninstalled, or hidden from this user; the package name still identifies it.
        }
        return fallback;
    }

    private PendingIntent managerIntent(
            String recordId, String action, int userId, int requestOffset) {
        Context userContext = contextAsUser(context, userId);
        Intent intent = new Intent("io.github.lingqiqi5211.crashcatcher.BRIDGE_ACTION")
                .setPackage(managerPackage)
                .putExtra("record_id", recordId)
                .putExtra("bridge_action", action)
                .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK | Intent.FLAG_ACTIVITY_CLEAR_TOP);
        int requestCode = 31 * recordId.hashCode() + requestOffset;
        return PendingIntent.getActivity(
                userContext, requestCode, intent,
                PendingIntent.FLAG_UPDATE_CURRENT | PendingIntent.FLAG_IMMUTABLE);
    }

    private void cancelNotification(String recordId) {
        NotificationManager manager = context.getSystemService(NotificationManager.class);
        if (manager != null) {
            manager.cancel(recordId, recordId.hashCode());
        }
    }

    private JSONObject queryPackageInfo(String packageName, int userId) throws Exception {
        Context userContext = contextAsUser(context, userId);
        PackageManager packageManager = userContext.getPackageManager();
        PackageInfo info = packageManager.getPackageInfo(packageName, 0);
        ApplicationInfo application = info.applicationInfo;
        JSONObject result = new JSONObject()
                .put("package_name", packageName)
                .put("user_id", userId)
                .put("version_name", nullable(info.versionName))
                .put("version_code", info.getLongVersionCode())
                .put("target_sdk", application == null ? JSONObject.NULL : application.targetSdkVersion)
                .put("min_sdk", application == null ? JSONObject.NULL : application.minSdkVersion)
                .put("primary_abi", application == null ? JSONObject.NULL : nullable(primaryAbi(application)))
                .put("label", application == null ? JSONObject.NULL : nullable(
                        packageManager.getApplicationLabel(application).toString()))
                .put("is_system_app", application != null
                        && (application.flags & ApplicationInfo.FLAG_SYSTEM) != 0);
        return result;
    }

    private void startActivity(JSONObject spec, int userId) throws JSONException {
        Intent intent = new Intent(spec.getString("action"));
        String packageName = nullableString(spec, "package_name");
        if (packageName != null) {
            intent.setPackage(packageName);
        }
        String component = nullableString(spec, "component");
        if (component != null) {
            int separator = component.indexOf('/');
            if (separator <= 0 || separator == component.length() - 1) {
                throw new IllegalArgumentException("component must be package/class");
            }
            String owner = component.substring(0, separator);
            String className = component.substring(separator + 1);
            // Accept the manifest shorthand as well as the qualified form. setClassName
            // takes the name literally — only manifest parsing expands a leading dot — so
            // a relative name silently names a class that does not exist and the launch
            // throws. Expanding it here means either spelling works.
            if (className.startsWith(".")) {
                className = owner + className;
            }
            intent.setClassName(owner, className);
        }
        JSONObject extras = spec.optJSONObject("extras");
        if (extras != null) {
            java.util.Iterator<String> keys = extras.keys();
            while (keys.hasNext()) {
                String key = keys.next();
                intent.putExtra(key, extras.getString(key));
            }
        }
        intent.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK);
        contextAsUser(context, userId).startActivity(intent);
    }

    private static JSONObject result(
            String type, long requestId, String booleanField, boolean value, String error)
            throws JSONException {
        JSONObject result = new JSONObject()
                .put("type", type)
                .put("request_id", requestId)
                .put(booleanField, value)
                .put("error", error == null ? JSONObject.NULL : error);
        if (type.equals("package_info_result")) {
            result.put("package", JSONObject.NULL);
        }
        return result;
    }

    private static Object nullable(String value) {
        return value == null ? JSONObject.NULL : value;
    }

    private static String nullableString(JSONObject object, String key) {
        return object.isNull(key) ? null : object.optString(key, null);
    }

    private static String primaryAbi(ApplicationInfo application) {
        try {
            java.lang.reflect.Field field = ApplicationInfo.class.getDeclaredField("primaryCpuAbi");
            field.setAccessible(true);
            Object value = field.get(application);
            return value instanceof String ? (String) value : null;
        } catch (ReflectiveOperationException ignored) {
            return null;
        }
    }

    private static UserHandle userHandle(int userId) {
        try {
            java.lang.reflect.Method method = UserHandle.class.getDeclaredMethod("of", int.class);
            method.setAccessible(true);
            return (UserHandle) method.invoke(null, userId);
        } catch (ReflectiveOperationException firstError) {
            try {
                java.lang.reflect.Constructor<UserHandle> constructor =
                        UserHandle.class.getDeclaredConstructor(int.class);
                constructor.setAccessible(true);
                return constructor.newInstance(userId);
            } catch (ReflectiveOperationException secondError) {
                secondError.addSuppressed(firstError);
                throw new IllegalStateException("Cannot create UserHandle", secondError);
            }
        }
    }

    private static Context contextAsUser(Context context, int userId) {
        try {
            java.lang.reflect.Method method = Context.class.getDeclaredMethod(
                    "createContextAsUser", UserHandle.class, int.class);
            method.setAccessible(true);
            return (Context) method.invoke(context, userHandle(userId), 0);
        } catch (ReflectiveOperationException error) {
            throw new IllegalStateException("Cannot create user context", error);
        }
    }

    private static JSONObject readFrame(DataInputStream input) throws IOException, JSONException {
        int length = input.readInt();
        if (length < 0 || length > MAX_FRAME_BYTES) {
            throw new IOException("invalid frame length " + length);
        }
        byte[] bytes = new byte[length];
        input.readFully(bytes);
        return new JSONObject(new String(bytes, StandardCharsets.UTF_8));
    }

    private static void writeFrame(DataOutputStream output, JSONObject object) throws IOException {
        byte[] bytes = object.toString().getBytes(StandardCharsets.UTF_8);
        if (bytes.length > MAX_FRAME_BYTES) {
            throw new IOException("frame too large");
        }
        output.writeInt(bytes.length);
        output.write(bytes);
        output.flush();
    }

    private static Context systemContext() throws Exception {
        Class<?> activityThread = Class.forName("android.app.ActivityThread");
        Method systemMain = activityThread.getDeclaredMethod("systemMain");
        systemMain.setAccessible(true);
        Object thread = systemMain.invoke(null);
        Method getSystemContext = activityThread.getDeclaredMethod("getSystemContext");
        getSystemContext.setAccessible(true);
        return (Context) getSystemContext.invoke(thread);
    }

    private static void dropToSystemUid() throws Exception {
        if (Process.myUid() == SYSTEM_UID) {
            return;
        }
        if (Process.myUid() != 0) {
            throw new SecurityException("bridge must start as root or system");
        }
        Os.setgid(SYSTEM_UID);
        Os.setuid(SYSTEM_UID);
    }

    private static final class Arguments {
        final String socketName;
        final String managerPackage;

        Arguments(String socketName, String managerPackage) {
            this.socketName = socketName;
            this.managerPackage = managerPackage;
        }

        static Arguments parse(String[] args) {
            String socket = null;
            String manager = null;
            for (int index = 0; index < args.length; index++) {
                if (args[index].equals("--socket") && index + 1 < args.length) {
                    socket = args[++index];
                } else if (args[index].equals("--manager-package") && index + 1 < args.length) {
                    manager = args[++index];
                }
            }
            if (socket == null || manager == null) {
                throw new IllegalArgumentException("--socket and --manager-package are required");
            }
            return new Arguments(socket, manager);
        }
    }
}
