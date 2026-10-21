// Export fixtures using the original Dynamic Draw assembly's own attributed types.
// This program never starts the plugin UI or writes its user-settings location.
using System.Collections;
using System.IO;
using System.Runtime.Loader;
using System.Runtime.Serialization;
using System.Text;
using System.Text.Json;
using System.Xml;

var plugin = Path.GetFullPath(args[0]);
var output = Path.GetFullPath(args[1]);
var install = Environment.GetEnvironmentVariable("PAINTDOTNET_DIR") ?? @"C:\Program Files\paint.net";
AssemblyLoadContext.Default.Resolving += (context, name) => {
    foreach (var dir in new[] {Path.GetDirectoryName(plugin)!, install}) {
        var path = Path.Combine(dir, name.Name + ".dll");
        if (File.Exists(path)) return context.LoadFromAssemblyPath(path);
    }
    return null;
};
var assembly = AssemblyLoadContext.Default.LoadFromAssemblyPath(plugin);
var settingsType = assembly.GetType("DynamicDraw.SettingsSerialization", true)!;
var brushType = assembly.GetType("DynamicDraw.BrushSettings", true)!;
var settings = Activator.CreateInstance(settingsType)!;
var brush = Activator.CreateInstance(brushType)!;
foreach (var (key, value) in new (string, object)[] {
    ("BrushSize", 8), ("BrushDensity", 2), ("AutomaticBrushDensity", false),
    ("BrushColor", -65536), ("BrushFlow", 128), ("BrushImagePath", "tip.png")
}) brushType.GetProperty(key)!.SetValue(brush, value);
((IDictionary)settingsType.GetProperty("CustomBrushes")!.GetValue(settings)!).Add("Original export", brush);
Directory.CreateDirectory(output);
File.WriteAllText(Path.Combine(output, "original.json"), JsonSerializer.Serialize(settings, settingsType), new UTF8Encoding(false));
using var xml = XmlWriter.Create(Path.Combine(output, "original.xml"), new XmlWriterSettings {Encoding = new UTF8Encoding(false)});
new DataContractSerializer(settingsType).WriteObject(xml, settings);
Console.WriteLine("Exported original Dynamic Draw " + assembly.GetName().Version + " JSON and XML settings.");
