// A real FileTypeFactory assembly, independent of the bridge implementation.
// The tiny format has its own magic and stores straight BGRA with integer dimensions.
using System.IO;
using PaintDotNet;

public sealed class FixtureFactory : IFileTypeFactory
{
    public FileType[] GetFileTypeInstances()=>[new FixtureType(false),new FixtureType(true)];
}
sealed class FixtureType(bool broken) : FileType(broken?"Broken fixture":"Craft test image",new FileTypeOptions {
    LoadExtensions=broken?[".pcbad"]:[".pcfixture",".pcread"],
    SaveExtensions=broken?[]:[".pcfixture",".pcwrite"]
})
{
    protected override Document OnLoad(Stream stream)
    {
        if(broken)throw new InvalidDataException("intentional decoder failure");
        using var r=new BinaryReader(stream);
        uint magic=r.ReadUInt32();
        // Exercise host termination during a decoder that does not offer cancellation itself.
        if(magic==0x574F4C53){Console.Error.WriteLine("fixture-waiting");Thread.Sleep(30000);magic=r.ReadUInt32();}
        if(magic!=0x54465354)throw new InvalidDataException("fixture signature");
        int w=r.ReadInt32(),h=r.ReadInt32();
        if(w<1||h<1||w>1000||h>1000)throw new InvalidDataException("fixture dimensions");
        var document=new Document(w,h) {DpuUnit=MeasurementUnit.Inch,DpuX=144,DpuY=192};
        var layer=new BitmapLayer(w,h);document.Layers.Add(layer);
        for(int y=0;y<h;y++)for(int x=0;x<w;x++)unsafe {layer.Surface[x,y]=ColorBgra.FromUInt32(r.ReadUInt32());}
        return document;
    }
    protected override void OnSave(Document document,Stream stream,SaveConfigToken token,Surface scratch,ProgressEventHandler progress)
    {
        document.Flatten(scratch);
        using var w=new BinaryWriter(stream,System.Text.Encoding.UTF8,true);
        w.Write(0x54465354u);w.Write(document.Width);w.Write(document.Height);
        for(int y=0;y<document.Height;y++)for(int x=0;x<document.Width;x++)w.Write(scratch[x,y].Bgra);
    }
}
