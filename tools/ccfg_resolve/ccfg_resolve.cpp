// ccfg_resolve — 078 レーン A のオラクル。
//
// 実 GET の CompoundConfig(reference/20190315_patched/CompoundConfig)で xcfg を読み、
// 与えられたパスの「有効値」(id="*" 継承を解決した後の値)を印字する。
// Rust レンダラ(src/params)の出力を、一次資料の解決器そのもので照合するために使う。
//
// 使い方:
//   ccfg_resolve <file.xcfg> [path ...]
//   ccfg_resolve <file.xcfg>            # パスを stdin から 1 行 1 個で読む
//
// パス構文は CompoundConfig の Path と同じ(ルート Setup は含めない):
//   Node[CoBo].Instance[0].AsAd[1].Aget[2].channel[5].LSBThresholdValue
//
// 出力は 1 行 1 パスのタブ区切り:
//   <path>\t<value>          解決できた
//   <path>\t!ERROR\t<what>   解決できなかった(stderr にも出す — silent failure を作らない)
//
// 終了コード: 0 = 全て解決 / 1 = 解決できないパスがあった / 2 = ファイルを読めなかった。

#include "CCfg/CConfig.h"
#include "CCfg/Io/Document.h"
#include "CCfg/Io/FileStorage.h"
#include "CCfg/Io/XmlFormat.h"
#include "CCfg/Object.h"
#include "CCfg/Value.h"

#include <exception>
#include <iostream>
#include <string>
#include <vector>

namespace {

// 1 パス分を解決して 1 行印字する。解決できたら true。
bool resolveOne(CCfg::Object& root, const std::string& path)
{
	try
	{
		CCfg::CConfig cfg(&root);
		CCfg::Attribute& attr = cfg.at(path.c_str()).find();
		std::cout << path << '\t' << attr.getValue().getAsString() << '\n';
		return true;
	}
	catch (const std::exception& e)
	{
		std::cout << path << "\t!ERROR\t" << e.what() << '\n';
		std::cerr << "ccfg_resolve: " << path << ": " << e.what() << std::endl;
		return false;
	}
}

} // namespace

int main(int argc, char** argv)
{
	if (argc < 2)
	{
		std::cerr << "usage: ccfg_resolve <file.xcfg> [path ...]" << std::endl;
		return 2;
	}

	// Document::loadFromFile は Url 経由で Format を選ぶが、その経路は使わない
	// (拡張子から XmlFormat を引けない環境があった)。Storage と Format を直に与える。
	// storage / format は Document より長生きさせる(Document は参照を保持する)。
	CCfg::Io::FileStorage storage(argv[1], CCfg::Io::FileStorage::Read);
	CCfg::Io::XmlFormat format;
	CCfg::Io::Document doc;
	CCfg::Object* root = 0;
	try
	{
		root = &doc.load(storage, format);
	}
	catch (const std::exception& e)
	{
		std::cerr << "ccfg_resolve: cannot load " << argv[1] << ": " << e.what() << std::endl;
		return 2;
	}

	std::vector<std::string> paths;
	for (int i = 2; i < argc; ++i)
	{
		paths.push_back(argv[i]);
	}
	if (paths.empty())
	{
		std::string line;
		while (std::getline(std::cin, line))
		{
			if (!line.empty() && line[line.size() - 1] == '\r')
			{
				line.erase(line.size() - 1);
			}
			if (!line.empty())
			{
				paths.push_back(line);
			}
		}
	}

	bool allOk = true;
	for (std::vector<std::string>::const_iterator it = paths.begin(); it != paths.end(); ++it)
	{
		allOk = resolveOne(*root, *it) && allOk;
	}
	return allOk ? 0 : 1;
}
