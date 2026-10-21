using System.Diagnostics;
using System.IO;
using System.Reflection;
using System.Reflection.PortableExecutable;
using System.Runtime.Loader;
using System.Text;
using Microsoft.Win32;
using PaintDotNet;
using PaintDotNet.Effects;

// Each assembly is inspected in a fresh process. One broken dependency, static
// initializer, crash or hang cannot prevent other assemblies from being listed.
internal static class PluginCatalog
{
    internal static Assembly Load(string path)
    {
        path=Path.GetFullPath(path);
        var dir=Path.GetDirectoryName(path)!;
        Environment.SetEnvironmentVariable("PATH",dir+Path.PathSeparator+Environment.GetEnvironmentVariable("PATH"));
        AssemblyLoadContext.Default.Resolving+=(c,n)=>{
            var p=Path.Combine(dir,n.Name+".dll");
            return File.Exists(p)?c.LoadFromAssemblyPath(p):null;
        };
        return AssemblyLoadContext.Default.LoadFromAssemblyPath(path);
    }
    internal static Type[] Types(Assembly assembly)
    {
        try {return assembly.GetExportedTypes();}
        catch(ReflectionTypeLoadException e) {
            foreach(var error in e.LoaderExceptions.Where(e=>e!=null)) Row("X",assembly.Location,"",error!.Message,"0","");
            return e.Types.Where(t=>t!=null).Cast<Type>().ToArray();
        }
    }
    internal static void Row(string kind,string path,string type,string name,string flags,string extensions)
    {
        // Hex-encoded UTF-8 fields keep paths, Unicode and error text unambiguous.
        Console.WriteLine(string.Join('\t',kind,Convert.ToHexString(Encoding.UTF8.GetBytes(path)),
            Convert.ToHexString(Encoding.UTF8.GetBytes(type)),Convert.ToHexString(Encoding.UTF8.GetBytes(name)),
            flags,Convert.ToHexString(Encoding.UTF8.GetBytes(extensions))));
    }
    internal static int Inspect(string path)
    {
        using(var stream=File.OpenRead(path)) using(var pe=new PEReader(stream)) if(!pe.HasMetadata)return 0;
        foreach(var t in Types(Load(path)).Where(t=>!t.IsAbstract && t.GetConstructor(Type.EmptyTypes)!=null)) {
            try {
                if(typeof(Effect).IsAssignableFrom(t)) {
                    using var effect=(Effect)Activator.CreateInstance(t)!;
                    Row("E",path,t.FullName!,effect.Name,"0","");
                } else if(typeof(EffectBase).IsAssignableFrom(t)) {
                    Row("U",path,t.FullName!,t.Name+": moderne Bitmap-/GPU-Host-API wird noch nicht unterstützt","0","");
                } else if(typeof(IFileTypeFactory).IsAssignableFrom(t)) {
                    var types=((IFileTypeFactory)Activator.CreateInstance(t)!).GetFileTypeInstances();
                    for(int i=0;i<types.Length;i++) FileRow(path,t.FullName+"#"+i,types[i]);
                } else if(typeof(FileType).IsAssignableFrom(t)) {
                    FileRow(path,t.FullName!, (FileType)Activator.CreateInstance(t)!);
                }
            } catch(Exception e) {Row("X",path,t.FullName!,e.GetBaseException().Message,"0","");}
        }
        return 0;
    }
    static void FileRow(string path,string type,FileType ft)
    {
        var options=ft.Options;
        var exts=new Dictionary<string,int>(StringComparer.OrdinalIgnoreCase);
        void Add(IEnumerable<string> extensions,int cap) {
            foreach(var e in extensions.Select(e=>e.TrimStart('.')).Where(e=>e.Length is >0 and <=29 && e.All(c=>char.IsAsciiLetterOrDigit(c)||c=='_'||c=='-'))) {
                exts.TryGetValue(e,out int flags);exts[e]=flags|cap;
            }
        }
        if(options.SupportsLoading)Add(options.LoadExtensions,1);
        if(options.SupportsSaving)Add(options.SaveExtensions,2);
        foreach(var group in exts.GroupBy(x=>x.Value))Row("F",path,type,ft.Name,(group.Key|(options.SupportsLayers?4:0)).ToString(),string.Join(';',group.Select(x=>x.Key)));
    }
    internal static int Scan(string[] args,string install)
    {
        var roots=new List<string>();
        for(int i=1;i+1<args.Length;i++) if(args[i]=="--root")roots.Add(args[++i]);
        if(!args.Contains("--isolated")) {
            var docs=Environment.GetFolderPath(Environment.SpecialFolder.MyDocuments);
            roots.Add(Path.Combine(docs,"PaintCraft App Files"));
            roots.Add(Path.Combine(docs,"Paint.NET App Files"));
            roots.Add(install);
            using var machine=Registry.LocalMachine.OpenSubKey(@"Software\Paint.NET");
            if(string.Equals(machine?.GetValue("Plugins/AllowAdditionalPluginDirectoryRoots")?.ToString(),"true",StringComparison.OrdinalIgnoreCase)) {
                using var user=Registry.CurrentUser.OpenSubKey(@"Software\Paint.NET");
                roots.AddRange((user?.GetValue("Plugins/AdditionalPluginDirectoryRoots")?.ToString()??"").Split(';',StringSplitOptions.RemoveEmptyEntries).Select(Environment.ExpandEnvironmentVariables));
            }
        }
        var seen=new HashSet<string>(StringComparer.OrdinalIgnoreCase);
        int count=0;
        foreach(var root in roots.Distinct(StringComparer.OrdinalIgnoreCase)) foreach(var folder in new[]{"Effects","FileTypes","Shapes"}) {
            var directory=Path.Combine(root,folder);
            if(!Directory.Exists(directory))continue;
            string[] files;
            try {files=Directory.GetFiles(directory,folder=="Shapes"?"*.xaml":"*.dll").Order(StringComparer.OrdinalIgnoreCase).ToArray();}
            catch(Exception e) {Row("X",directory,"",e.Message,"0","");continue;}
            foreach(var file in files) {
                var full=Path.GetFullPath(file);
                if(!seen.Add(full))continue;
                if(folder=="Shapes") {Row("S",full,"",Path.GetFileNameWithoutExtension(full),"0","");continue;}
                if(++count>256){Row("X",full,"","Maximal 256 Plugin-DLLs pro Start","0","");return 0;}
                try {
                    using(var stream=File.OpenRead(full)) using(var pe=new PEReader(stream)) if(!pe.HasMetadata)continue;
                    var executable = Environment.ProcessPath ?? "dotnet";
                    var start=new ProcessStartInfo(executable) {UseShellExecute=false,CreateNoWindow=true,RedirectStandardOutput=true,RedirectStandardError=true};
                    if (Path.GetFileNameWithoutExtension(executable).Equals("dotnet", StringComparison.OrdinalIgnoreCase))
                        start.ArgumentList.Add(Assembly.GetExecutingAssembly().Location);
                    start.ArgumentList.Add(full);start.ArgumentList.Add("--inspect");
                    HostWatchdog.Forward(args, start);
                    start.Environment["DOTNET_ReadyToRun"]="0";
                    using var child=Process.Start(start)!;
                    var stdout=ReadLimited(child.StandardOutput);var stderr=ReadLimited(child.StandardError);
                    if(!child.WaitForExit(15000)){child.Kill(true);child.WaitForExit();Row("X",full,"","Zeitlimit beim Laden (15 Sekunden)","0","");continue;}
                    if(child.ExitCode==0)Console.Write(stdout.GetAwaiter().GetResult());
                    else Row("X",full,"",stderr.GetAwaiter().GetResult(),"0","");
                } catch(Exception e) {Row("X",full,"",e.GetBaseException().Message,"0","");}
            }
        }
        return 0;
    }
    static async Task<string> ReadLimited(StreamReader reader)
    {
        var text=new StringBuilder();var buffer=new char[4096];int n;
        while((n=await reader.ReadAsync(buffer))!=0) {if(text.Length<1_000_000)text.Append(buffer,0,Math.Min(n,1_000_000-text.Length));}
        return text.ToString();
    }
}
