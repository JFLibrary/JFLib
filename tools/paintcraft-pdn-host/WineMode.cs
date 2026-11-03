using System.IO;
using System.Reflection;
using System.Text.Json;

// Paint.NET-on-Wine replaces what Wine lacks with managed implementations, and paintdotnet.exe installs
// them at its start when its configuration says EnableWineMode. The helper starts instead of that program,
// so it does the same. These members exist only in that package, not in the Paint.NET the helper is built
// against, so they are reached by name.
internal static class WineMode
{
    const BindingFlags Any=BindingFlags.Static|BindingFlags.Public|BindingFlags.NonPublic;

    internal static void Apply(string dir)
    {
        var path=Path.Combine(dir,"PaintDotNet.Configuration.json");
        if(!File.Exists(path))return;
        using var json=JsonDocument.Parse(File.ReadAllText(path));
        var root=json.RootElement;
        if(!root.TryGetProperty("EnableWineMode",out var on) || on.ValueKind!=JsonValueKind.True)return;
        Type.GetType("PaintDotNet.Imaging.ManagedImagingComponents, PaintDotNet.Windows.Imaging.Managed",true)!.GetMethod("Install",Any)!.Invoke(null,null);
        Set(Type.GetType("PaintDotNet.Dxgi.DxgiFactory7, PaintDotNet.Windows",true)!,"UseAlternateWarpLookup",true);
        var write=typeof(PaintDotNet.DirectWrite.DirectWriteFactory);
        Set(write,"UseDWriteCore",true);
        foreach(var name in new[]{"DWriteCoreFallbackFontFileName","DWriteCoreFallbackFontFamilyName"})
            if(root.TryGetProperty(name,out var value))Set(write,name,value.GetString());
        Type.GetType("PaintDotNet.Direct2D1.Managed.ManagedDirect2DFactory, PaintDotNet.Windows.Direct2D1.Managed",true)!.GetMethod("InstallAsDefaultDirect2DBackend",Any)!.Invoke(null,null);
    }

    static void Set(Type type,string property,object? value)=>type.GetProperty(property,Any)!.SetValue(null,value);
}
